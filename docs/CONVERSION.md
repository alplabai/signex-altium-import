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
  placements gets numbered variants (`import:R~2`).
- **PCB:** mils, Y up, absolute coordinates map to mm, Y down, with the
  board outline's top-left at (0, 0). Pads and footprint graphics are
  footprint-local, matching how the Signex renderer places them: Signex
  mirrors local x on a bottom-side footprint, so the local coordinates
  written for one undo that mirror as well as the footprint's turn.
- **Turns:** Altium turns counter-clockwise with Y up; on Signex's Y-down
  board the same visual turn is negative. A footprint's turn `c` becomes
  `-c`, a free text's turn `t` becomes `-t`, and a text inside a footprint
  gets the own turn `c - t` on top and `t - c` on the bottom (where
  Signex counts own turns the other way).
- **Names:** the output names no source tool. The generator is
  `signex-import`, a symbol's library id is `import:<library reference>`,
  and a free pad's holder footprint is `import:free-pad`.
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
| Component | `Footprint`, rotation negated for the Y flip. Designator and comment come from its special strings; the component's unique id is dropped (reported). |
| Pad | `Pad`. Plated and drilled → thru, unplated → np-thru, otherwise SMD. Rounded-rectangle radius → `roundrect_ratio` = percent / 200. |
| Copper layers | The board's stack top to bottom: `TopCopper`, `Inner1Copper` .. in stack order (mid layers and internal planes alike), `BottomCopper`; planes are typed `plane`. A board of more than 32 copper layers is refused. A mid or plane layer outside the stack is `Unmapped<n>` (reported); only a library, which has no stack, names a mid layer `Inner<n>Copper` by its number. |
| Internal plane | A pour (`fill_type = "pour"`) over the board outline, pulled back from the edge by `PLANEnPULLBACK`, on the plane's net (`PLANEnNETNAME`); a plane with no net is written without one (reported). Split lines on a plane are reported, not imported: a split plane is one pour on its plane net, and the plane areas of its other nets are lost (reported). |
| Fill or free region on an internal plane | A void in the plane's pour: a keepout zone on that plane (reported) |
| Track on copper | `Segment`. Tracks on other layers become board or footprint graphics. |
| Arc on copper | Straight segments, 10° per chord (reported). Other arcs become graphics. |
| Via | `Via`. Blind or buried when it does not span top to bottom. |
| Polygon pour | A pour (`fill_type = "pour"`) with the polygon's outline and net; Signex refills it. Altium pours the lowest `POURINDEX` first, Signex fills the highest priority first, so priority = the board's last pour index − `POURINDEX`. The clearance defers to the board's rule. |
| Fill on copper, free copper region | Solid copper (`fill_type = "solid"`) |
| Fill on another layer | A `poly` board drawing |
| Keepout fill or region (flagged keepout, or on the keep-out layer, also inside a footprint) | Keepout zones (`fill_type = "keepout"`): one per copper layer for the keep-out layer, else on the item's own copper layer. Per-object restrictions (tracks, vias, pads) become a keepout of all copper, and a footprint's keepout becomes a board keepout that does not move with the footprint; both are reported. |
| Keepout lines and arcs | A keepout line keeps copper clear along the line only: each segment (each 10° chord of an arc) becomes a thin keepout zone at the line's width with round ends, on the layers a keepout region would use. A closed loop of keepout lines keeps its inside free. A line with no width is reported, not imported. |
| Region with fewer than 3 points, footprint region that is not a keepout | Reported, not imported |
| Arc on another layer, outside a footprint | A board arc from start to end with the angle rising in the Y-down board frame, so start and end swap for the Y flip |
| Mechanical layer | Courtyard, assembly (`*Fab`) or outline by its name, else `MechanicalN` |
| Drill drawing, drill guide | `DrillDrawing`, `Notes` |
| Any other layer | `Unmapped<n>`, reported |
| Board shape | board outline (arc edges flattened) |

**Approximations:**

- A pad turned against its footprint keeps quarter turns only, as a
  width/height swap.
- Octagonal pads become rectangles.
- Hatched pours become solid.

**Not imported:** 3D bodies, dimensions, board cutouts, design rules and
classes. Recheck clearances in Signex before you send a board out.

## Output

The command converts every document of a project before it writes any of
them, so a board that is refused writes nothing. An I/O error while
writing can still leave the files written before it. A single board
(`.PcbDoc`) also reports a placement self-check in `--json`: how many
track ends land on a pad of their own net and how many only on another
net's pad (see `docs/APP_INTEGRATION.md`).

The vendored Signex types are still those of the format's 1.1 revision
(no pad rotation column, no `remove_dead_islands`); moving them to 1.4 is
a follow-up.

## Library conversion

`signex-altium-import library <input>... --output <dir>` turns `.SchLib` and
`.PcbLib` files into one Signex library (`docs/LIBRARY_FORMAT.md` in the
Signex repository is the format; `docs/APP_INTEGRATION.md` the command).

```text
<dir>/
  <name>.snxlib             manifest and one table per part class
  symbols/<name>.snxsym     one envelope per SchLib, every symbol in it
  footprints/<uuid>.snxfpt  one footprint per file (Signex reaches only
                            the first in a file)
  import-report.json, import-report.md
```

- **Ids** are derived (UUIDv5), never random. The library id comes from the
  input file names (lower case, sorted), so moving or reordering inputs keeps
  it; `--library-id` overrides it. Symbol, footprint and part ids come from
  the library id, the source library's name and the item's name; a repeated
  name inside one library adds `#2`, `#3`.
- **Same inputs, same bytes.** Every written time is the Unix epoch, and
  every primitive and part starts at version `1.0.0`. The converter writes
  through the vendored serializers and re-reads its own `.snxlib` to check it
  is canonical.
- **One part per SchLib component**, in (source path, source order). Part
  number = the component name; the second and later parts with the same name
  are `name~2`, `name~3` (report `part.pn_collision`). State `draft`. The
  parameter `designator_prefix` holds the designator's leading letters
  (`R?` gives `R`), when it has any.
- **Class** from the designator prefix, limited to the classes the vendored
  templates know:

  | Prefix | Class |
  |---|---|
  | `R` | `resistor` |
  | `C` | `capacitor` |
  | `L` | `inductor` |
  | anything else (`D`, `LED`, `Q`, `U`, `IC`, `J`, `P`, `CN`, `SW`, `S`, `Y`, `X`, `H`, `MH`, `FID`, none) | `generic` |

  One table per class used, named by the class key; the manifest lists those
  classes. More classes follow when templates exist for them.
- **Footprints** keep the footprint converter's frame: Signex's footprint
  frame has +y down, so every Altium y and angle is negated and each arc's
  start and end swap. Every footprint of every PcbLib is written; parts bind
  to them by name, case-insensitive, the first definition winning
  (`footprint.name_collision` for a repeat).
- **Footprint binding.** A component's PCB model marked current is the
  default footprint; its other PCB models are alternates in list order. When
  none is marked current the first is the default (`part.footprint_no_current`).
  A model whose pin map renames pins gives `pin_map_overrides` (default) or
  the alternate's own overrides; identical pairs are not overrides. A named
  footprint that is in none of the given PcbLibs skips the **part** with
  `part.footprint_missing` (`error`). A component that names no footprint
  gets a row without one. Models of other kinds (a simulation, for example)
  are reported as `info` (`part.model_skipped`) and not imported. Pin maps are
  not read from files yet (`docs/FORMAT_NOTES.md`).
- **`--update`** needs the directory to hold exactly one `.snxlib`, with the
  same library id as this source gives (else the run fails and writes
  nothing). It rewrites the library in place and deletes envelopes,
  footprints and library files the source no longer has. A part keeps its
  lifecycle state, `released` flag and created time; its version and updated
  time stay while its content hash is unchanged and otherwise the patch
  version rises and updated is set to now. Symbol and footprint versions stay
  `1.0.0`.
- **Not carried yet:** embedded 3D models, `.IntLib` and `.DbLib` sources,
  folder inputs, simulation models.
- **Completeness.** Every source component is a written part or a part item
  with severity `error`; the run fails with an internal error if the report
  does not account for every one.

## Report codes

Every approximation or loss has a stable code, `<area>.<what>`; a code never
changes meaning. The single-file `--json` summary groups items by message and
lists all of them; the library report (`import-report.json`) keeps each item
with its code, severity and source. Severities are the library converter
contract's: `info` (nothing lost), `approximated` (written, approximately),
`dropped` (written, part left out), `error` (item not written).

| Code | Severity | Meaning |
|---|---|---|
| `symbol.pin_ieee_unmapped` | dropped | A pin's IEEE symbol code with no Signex shape (detail `ieee_code`) |
| `symbol.bezier_polygon` | approximated | Library symbol bezier imported as its control polygon |
| `symbol.rounded_rect_square` | approximated | Library symbol rounded rectangle imported with square corners |
| `sch.rounded_rect_square` | approximated | Sheet rounded rectangle imported with square corners |
| `sch.bezier_polygon` | approximated | Sheet bezier imported as its control polygon |
| `sch.image_dropped` | dropped | Embedded image not imported |
| `sch.record_dropped` | dropped | Other schematic record type not imported (detail `record`) |
| `footprint.courtyard_text_dropped` | dropped | Text on a courtyard layer not imported |
| `footprint.graphic_layer_unmapped` | dropped | Footprint graphic on a layer with no slot not imported (detail `layer`) |
| `footprint.copper_track_dropped` | dropped | Copper track inside a library footprint not imported |
| `footprint.fill_dropped` | dropped | Fill inside a library footprint not imported |
| `footprint.region_dropped` | dropped | Region (custom pad shape) inside a library footprint not imported |
| `footprint.courtyard_open_bbox` | approximated | Open courtyard outline replaced by its bounding box |
| `footprint.pad_octagon_rect` | approximated | Octagonal library pad imported as a rectangle |
| `pcb.keepout_line_no_width` | dropped | Keepout line or arc with no width not imported |
| `pcb.keepout_non_copper` | dropped | Keepout on a non-copper layer not imported |
| `pcb.footprint_keepout_moved` | approximated | Footprint keepout becomes a board keepout that does not move with the footprint |
| `pcb.keepout_restrictions_merged` | approximated | Per-object keepout restrictions become a keepout of all copper |
| `pcb.component_uid_dropped` | dropped | Source-tool component unique id dropped |
| `pcb.free_pad_holder` | approximated | Free pad imported into its own footprint |
| `pcb.plane_split_line_dropped` | dropped | Plane split line not imported |
| `pcb.copper_arc_flattened` | approximated | Copper arc imported as straight track segments |
| `pcb.plane_void_keepout` | approximated | Fill or region on an internal plane becomes a keepout void |
| `pcb.pour_hatch_solid` | approximated | Hatched polygon pour imported as a solid pour |
| `pcb.region_too_few_points` | dropped | Region with fewer than 3 points not imported |
| `pcb.footprint_region_dropped` | dropped | Footprint region (custom pad shape or copper) not imported |
| `pcb.region_cutout_dropped` | dropped | Cutout or non-copper region not imported |
| `pcb.body_dropped` | dropped | 3D component body not imported |
| `pcb.primitive_dropped` | dropped | Other PCB primitive type not imported (detail `primitive`) |
| `pcb.plane_no_outline` | dropped | Plane not written: the board has no outline |
| `pcb.split_plane_nets_lost` | dropped | Split plane written as one pour; other nets' areas lost |
| `pcb.plane_no_net` | approximated | Plane with no net written as a pour without a net |
| `pcb.pullback_not_applied` | approximated | Plane pullback could not be applied; the pour covers the outline |
| `pcb.layer_unmapped` | approximated | Source layer with no Signex layer; its items are on `Unmapped<n>` (detail `layer`) |
| `pcb.pad_rotation_dropped` | approximated | Pad turned off a quarter turn from its footprint imported unrotated |
| `pcb.pad_octagon_rect` | approximated | Octagonal pad imported as a rectangle |
| `project.several_boards` | info | Project has several PCB documents; the first is the project's board |
| `part.pn_collision` | approximated | A part number was taken; the part got a `~n` suffix |
| `part.footprint_missing` | error | A named footprint is in none of the given PcbLibs; part skipped |
| `part.footprint_no_current` | info | No footprint is marked current; the first is the default |
| `part.model_skipped` | info | A model of another kind, or a footprint bound twice, not imported |
| `part.param_ambiguous` | info | A parameter value kept as text or not imported because it is unclear |
| `part.param_dropped` | dropped | A parameter without a usable name, or one that repeats another |
| `part.text_sanitized` | info | Control characters or a triple apostrophe in a value replaced |
| `part.template_incomplete` | info | The class template wants parameters the part lacks |
| `part.row_invalid` | error | The part row failed the writer's checks; part not written |
| `footprint.name_collision` | info | A footprint name is already defined; parts bind to the first |
| `library.input_unreadable` | error | An input file could not be read (library runs) |

48 codes. The catalogue lives in `crates/signex-altium-convert/src/codes.rs`.
