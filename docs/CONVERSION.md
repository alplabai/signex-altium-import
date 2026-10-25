# Conversion rules

How Altium concepts land in Signex, and where the result is approximate.
Each approximation is also counted in the run's report.

## Frames

- **Schematic:** Altium mils, Y up, from the bottom-left of the sheet map
  to Signex mm, Y down, from the top-left: `(x, H - y)` with `H` the Altium
  sheet height. The paper size is the nearest Signex paper.
- **Symbols:** each placed component's children are pulled back into
  symbol-local coordinates and embedded as a `lib_symbol`. Signex's
  `SymbolTransform` then reproduces every point exactly; the unit tests
  check all 8 rotation and mirror combinations. Identical symbols share
  one `lib_symbol`. A library reference whose geometry differs between
  placements gets numbered variants (`altium:R~2`).
- **PCB:** mils, Y up, absolute coordinates map to mm, Y down, with the
  board outline's top-left at (0, 0). Pads and footprint graphics are
  footprint-local, matching how the Signex renderer places them.
- **Lengths** are rounded to 10 nm. Altium's internal unit is 2.54 nm, so
  rounding recovers mm-designed values exactly.

## Schematic

| Altium | Signex |
|---|---|
| Component | `Symbol` and an embedded `LibSymbol`. Designator → reference, `Value` (else resolved `Comment`) → value, current PCB footprint model → footprint, the other parameters → fields. |
| Part *n* of a multi-part component | `unit` *n*. Graphics with `OwnerPartId` -1 are shared (unit 0). |
| Pin | Pin at its electrical end, rotated to point at the body. Electrical type → `PinDirection`; clock and dot → `PinShapeStyle`. |
| Wire, bus, bus entry, junction | the same object |
| Net label | `Label` (net) |
| Power port | Power `Symbol` with `lib_id` `power:<style>` and the net name as value. |
| Off-sheet connector | global `Label` |
| Port | `Label`, hierarchical when the design has sheet symbols, otherwise global. Override with `--ports`. Placed at whichever end of the port something touches. |
| Sheet symbol and entries | `ChildSheet` and `SheetPin`, file name rewritten to the converted `.snxsch` |
| No ERC | no-ERC directive |
| Lines, polylines, polygons, rectangles, arcs, ellipses, text | drawings and text notes |
| Sheet parameters | title block (`title`, `rev`, `date`, `company`, …), and the source for `=Name` special strings |

**Contact connections.** Altium connects objects that merely touch: a
power port dropped on a pin, two pins tip to tip, a label on a pin.
Signex connects only through wires. The converter adds a zero-length wire
at every point where two or more connectable objects meet without a wire,
so connectivity survives and the drawing stays exactly as it was.

**Not imported:** images, harnesses, the sheet template's artwork
(Signex draws its own title block), rounded-rectangle corner radii
(imported square), and alternate display modes other than the one in use.

## Libraries

A `.SchLib` becomes one `.snxsym` holding every symbol, and a `.PcbLib`
becomes one `.snxfpt` holding every footprint. Both keep the library's own
order and names.

- **Symbols** stay Y up, local to their origin:
  - **Pins:** at their electrical end, facing the body.
  - **Units:** one per Altium part (part -1 is shared).
  - **Fields:** designator and comment, with their positions. The other
    parameters, plus the current footprint model, become schematic
    parameters.
  - **Decorations:** the dot and clock IEEE marks map. Other IEEE codes
    are reported.
- **Footprints** flip to the footprint editor's Y-down frame, so rotations
  negate:
  - **Pads:** each keeps its own rotation. Layers use the editor's
    new-pad names (`F.Cu` / `F.Mask` / `F.Paste`, `*.Cu` for through-hole).
  - **Graphics:** overlay goes to silkscreen. Assembly layers go to fab.
  - **Courtyard:** the courtyard layer becomes the courtyard outline. If it
    doesn't close, its bounding box is used, and that is reported.
  - **Layer roles:** the file's `LayerKindMapping` decides each mechanical
    layer's role first, and the layer's name second.
- **Not imported:** copper drawn inside a footprint, custom pad shapes
  (regions), fills, 3D bodies.

## PCB

| Altium | Signex |
|---|---|
| Component | `Footprint`, rotation negated for the Y flip. Designator and comment come from its special strings. |
| Pad | `Pad`. Plated and drilled → thru, unplated → np-thru, otherwise SMD. Rounded-rectangle radius → `roundrect_ratio` = percent / 200. |
| Track on copper | `Segment`. Tracks on other layers become board or footprint graphics. |
| Arc on copper | Straight segments, 10° per chord (reported). Other arcs become graphics. |
| Via | `Via`. Blind or buried when it does not span top to bottom. |
| Polygon pour | `Zone` with the polygon's outline. The pour is recomputed by Signex. |
| Fill on copper, free copper region | `Zone` |
| Mechanical layer | Courtyard, assembly (`*Fab`) or outline by its name, else `MechanicalN` |
| Board shape | board outline (arc edges flattened) |

**Approximations:**

- A pad turned against its footprint keeps quarter turns only, as a
  width/height swap.
- Octagonal pads become rectangles.
- Hatched pours become solid.

**Not imported:** 3D bodies, dimensions, board cutouts, design rules and
classes. Recheck clearances in Signex before you send a board out.
