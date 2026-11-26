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
  - **Decorations:** the dot and clock IEEE marks map (outer and inner edge
    slots). Every other IEEE code, in any of the four slots, is reported
    with its code and slot (`symbol.pin_ieee_unmapped`); the file's codes
    for the other marks are not known from any sample, so none is guessed.
  - **Shapes:** a rounded rectangle keeps its corner radius and a cubic
    Bezier is written as a Bezier (symbol format 4).
- **Footprints** flip to the footprint editor's Y-down frame, so rotations
  negate:
  - **Pads:** each keeps its own rotation. Layers are the named
    tokens of the library format: `Top Layer|Top Solder|Top Paste` for a top
    SMD pad (`Bottom …` for a bottom one), `Top Layer|Bottom Layer|Top
    Solder|Bottom Solder` for through-hole, `Top Solder|Bottom Solder` for an
    unplated hole.
  - **Graphics:** overlay goes to silkscreen. Assembly layers go to fab.
  - **Courtyard:** the courtyard layer becomes the courtyard outline. If it
    doesn't close, its bounding box is used, and that is reported.
  - **Layer roles:** the file's `LayerKindMapping` decides each mechanical
    layer's role first, and the layer's name second.
- **Display modes** become body styles: mode 0 is the normal body
  (`body_style` 1), mode 1 the alternate (De Morgan) body (2), an item for no
  particular mode (-1) belongs to both (0). Items of a further mode are
  dropped and reported (`symbol.display_mode_dropped`).
- **Pin decorations:** schematic placement reads only the outside-edge column,
  so the dot (outer) and a lone clock go there. A pin with both keeps the dot
  there and the clock in the inside-edge column, which placement ignores
  (`symbol.pin_decoration_merged`, approximated).
- **Text in cells:** control characters in pin names, numbers and
  descriptions, pad numbers and part names become spaces, and a triple
  apostrophe three typographic quotes, since the writer refuses them
  (`symbol.text_sanitized`, `footprint.text_sanitized`, `part.text_sanitized`).
- **Pads:** an octagonal pad is a custom outline with regular-octagon cuts (the board refuses chamfered pads)
  (`footprint.pad_octagon_custom`).
- **Not imported, each with its own code:** copper tracks
  (`footprint.copper_track_dropped`), fills (`footprint.fill_dropped`),
  regions, which include custom pad shapes (`footprint.region_dropped`), and
  graphics on layers the format has no list for
  (`footprint.graphic_layer_unmapped`). The library format has no home for
  copper graphics or custom regions beyond a pad's own outline, and turning
  a region into a pad's custom outline would need the pad-and-region union
  rule, which no sample shows.

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
- **A source saved again** without changes gives the same library: an update keeps an envelope's text, times included, when only its own times differ.
- **Same inputs, same bytes.** Every written time is the newest
  modified time recorded in the source containers' root entries (whole
  seconds, from the input bytes, never the clock), or the Unix epoch when no
  source records one. Every primitive and part starts at version `1.0.0`. The converter writes
  through the vendored serializers and re-reads its own `.snxlib` to check it
  is canonical.
- **One part per SchLib component**, in (source path, source order). Part
  number = the component name; the second and later parts with the same name
  are `name~2`, `name~3` (report `part.pn_collision`). State `draft`. The
  parameter `designator_prefix` holds the designator's leading letters
  (`R?` gives `R`), when it has any.
- **Class** from the designator prefix, with the stock library's class keys
  and labels:

  | Prefix | Class (label) |
  |---|---|
  | `R` | `resistor` (Resistor) |
  | `C` | `capacitor` (Capacitor) |
  | `L` | `inductor` (Inductor) |
  | `D` | `diode` (Diode) |
  | `LED` | `led` (LED) |
  | `Q` | `transistor` (Transistor) |
  | `U`, `IC` | `ic` (Integrated Circuit) |
  | `J`, `P`, `CN` | `connector` (Connector) |
  | `SW`, `S` | `switch` (Switch) |
  | `Y`, `X` | `crystal` (Crystal / Oscillator) |
  | `H`, `MH` | `mechanical` (Mechanical) |
  | `FID` | `fiducial` (Fiducial) |
  | anything else, or none | `generic` (Generic) |

  One table per class used, named by the class key; the manifest declares
  every class used. Only `resistor`, `capacitor` and `inductor` have a
  parameter template today; the others use the generic template's slots.
- **Footprints** keep the footprint converter's frame: Signex's footprint
  frame has +y down, so every Altium y and angle is negated and each arc's
  start and end swap. Every footprint of every PcbLib is written; parts bind
  to them by name, case-insensitive, the first definition winning
  (`footprint.name_collision`, approximated, names both definitions).
- **Footprint binding.** A component's PCB model marked current is the
  default footprint; its other PCB models are alternates in list order. When
  none is marked current the first is the default (`part.footprint_no_current`).
  A model whose pin map the reader cannot decode (a map record with content) skips the part with `part.pin_map_unread` (`error`) rather than binding it 1:1. A model whose pin map renames pins gives `pin_map_overrides` (default) or
  the alternate's own overrides; identical pairs are not overrides. A named
  footprint that is in none of the given PcbLibs skips the **part** with
  `part.footprint_missing` (`error`). A component that names no footprint
  gets a row without one. Models of other kinds (a simulation, for example)
  are reported as `info` (`part.model_skipped`) and not imported. Pin maps are
  not decoded yet (`docs/FORMAT_NOTES.md`), so no part gets overrides from
  a file; alternates carry their footprint's version.
- **`--update`** needs the directory to hold exactly one `.snxlib`, with the
  same library id as this source gives (else the run fails and writes
  nothing). It rewrites the library in place and deletes envelopes,
  footprints and library files the source no longer has. Each symbol and
  footprint is compared with its file on disk: unchanged ones keep their
  version and times, changed ones get the next patch version and `updated` =
  now. A part keeps its lifecycle state, `released` flag and created time;
  its `symbol_version`/`footprint_version` follow the primitives, and its
  own patch version rises (with `updated` = now) when its content hash or
  either primitive version changed. A part row, symbol or footprint on disk
  that cannot be read fails the run before anything is written.
- **Integrated libraries (`.IntLib`)** contribute their embedded schematic
  and footprint libraries as if they had been given separately, under the
  `.IntLib`'s name, and their parts are `Released` (state and `released`
  flag, on symbols and footprints too). The cross-reference stream is only a
  cross-check: a different entry count or a component name it lacks is
  reported (`library.crossref_mismatch`, info). An `.IntLib` and a
  `.SchLib` or `.PcbLib` of the same name cannot be given together.
- **3D models.** A footprint's component body names a model of the library's
  model table. A STEP model becomes the footprint's `step_attachment` and a
  `[[models]]` entry in the footprint's own file (format 1.4); its hash is the
  SHA-256 of the inflated bytes. Placement: Signex turns a model about x, then
  y, then z in its own y-up frame, adds the offset, and then flips into the
  footprint's y-down frame, and Altium's placement is y-up already, so the
  offset is (`MODEL.2D.X`, `MODEL.2D.Y`, `MODEL.3D.DZ`) in mm and the rotation
  (`ROTX`, `ROTY`, `ROTZ` + `MODEL.2D.ROTATION`) in degrees, unchanged
  (`LIBRARY_FORMAT.md` 4.3 states Signex's model frame). That Altium's turn
  order equals Signex's is not verified in a 3D view; a body with two or more
  non-zero turns, a 2D rotation or a bottom-side projection is reported
  (`model.placement_approximated`). `--models-dir DIR` supplies a model a body names but the file does not hold: the file is looked up in that one folder by name (any case, also with `.step` or `.stp` added), as a plain file of at most 256 MiB charged to the run budget. `--models skip` embeds none and reports
  each (`model.skipped`, info; skipped models are not counted). A body that does not say its model is
  embedded and has no table entry names a model stored outside the file
  (`model.external`; observed in a private corpus, where no integrated
  library stored its 3D models). A model that
  is not STEP, not embedded, unreadable or missing from the table, and a
  second model on one footprint, are not written and reported as `error`
  items of kind `model`. A body without a model is reported as info.
- **Update rules.** The report's `ledger` lists each file written with its
  SHA-256. `--update` deletes a ledgered file only when it left the source
  and its bytes still match; a file a user edited stays. A target file that is
  not in the ledger and differs is never overwritten: a symbol envelope takes
  its uuid name instead, any other target stops the run before anything is
  written. A ledgered primitive that a user's row still references is kept
  (`library.kept_referenced`) and stays in the ledger. A part number a user
  row already holds stops the run. The library name is kept unless `--name`
  is given. A stale name that is the same file as a written one (another
  case, or the same inode) is never deleted.
- **Limits.** One run inflates at most 2 GiB of embedded data, and a stream
  may not inflate to more than 200 times its size (1 MiB always allowed).
- **Not carried yet:** `.DbLib` sources, folder inputs, simulation models.
- **Completeness.** Every source component is a written part or a part item
  with severity `error`; the run fails with an internal error if the report
  does not account for every one.

### Part fields

Parameters of the component map to the part's fields. Names match without
case, spaces or underscores; a trailing number picks the pair (`Supplier 2`,
no number means 1).

| Altium parameter | Part field |
|---|---|
| `Manufacturer n`, `Manufacturer Part Number n` (also `MPN n`) | manufacturer part n; the lowest n is `primary_mpn` (status Primary), the others `alternates` (Approved) in n order. Either half alone gives a part with the other blank. |
| `Supplier n`, `Supplier Part Number n`, `Supplier Order Qty n` | supply listing n (distributor, SKU, `moq`). A supplier without a part number, or the reverse, is not imported (`part.param_ambiguous`). A quantity that is not a whole number is not imported. |
| `Datasheet` | the datasheet, unlabelled |
| `HelpURL`, `ComponentLinkNURL` | more datasheets, labelled with the parameter name, or with `ComponentLinkNDescription` when there is one. Without `Datasheet`, the first of these is the primary document and keeps its label. |
| `Description`, else the component's description | `description` |
| `Comment` | not a part field (the symbol keeps its comment) |
| a parameter the class template has a slot for (`Value`, `Tolerance`, `Power`, `Voltage`, `Current`, `Package`, `Dielectric`, `ESR`, `DCR`, `TCR`, `SRF`, `Rating`, with the usual spellings) | the slot's parameter name. A measurement slot gets a **measurement** only when the text is a number followed by a unit the slot accepts (`4.7 kOhm`, `0.1W`, `1%`, `100ppm/C`), stored in the base unit. A bare number, a bare prefix (`10k`), another unit or extra text stays text and is reported `info` (`part.param_ambiguous`). |
| every other parameter | a text parameter named as in the source |

Blank values and `*` are skipped. Control characters in a value become spaces
and a triple apostrophe becomes three typographic quotes, because the table
is a TOML literal string (`part.text_sanitized`, `info`). A repeated name
keeps the later value (`part.param_dropped`). When the class template
requires parameters the part lacks, one `info` item lists them
(`part.template_incomplete`).

## Behaviour changes

- **`~n` part numbers.** A repeated key gets `~2`, `~3` in a fixed order that
  does not depend on the database's physical row order (a compacted
  database reorders rows): rows are ordered by key and then by every cell,
  tables by name. A generated number is reserved, so it cannot collide with a
  later source name.
- A schematic library component whose storage cannot be read is skipped and
  reported (`symbol.unreadable`); it no longer fails the whole library.
- The single-file warning for a footprint graphic on a layer with no list now
  reads "graphic on <layer> dropped: Signex footprints have no such layer yet
  (signex #3133)" (the `--json` summary message changed; the code and the
  count did not).
- Footprint pads carry the library format's named layer tokens (`Top Layer|Top
  Solder|Top Paste`, ...), not the editor's (`F.Cu`, ...), in every
  `.snxfpt` this tool writes, the single-file `.PcbLib` conversion included:
  the editor spelling made a footprint unplaceable on a board.

## Database libraries

`signex-altium-import library Parts.DbLib --output <dir>` converts a database
library: its rows become parts, and the schematic and footprint libraries the
rows name supply their symbols and footprints. A `.DbLib` is given on its own.

- **Tables.** Each enabled table becomes one library table named after it; a
  disabled table is reported (`table.disabled`, info). A table the database
  does not hold is reported (`table.missing`, error).
- **Key.** The column whose field map has `FieldType=0` is the part number.
  A repeated key gets `~2`, `~3` (`part.pn_collision`). A row without a key
  value is skipped (`row.key_missing`); a table with no key field has all its
  rows skipped (`table.no_key`).
- **Columns.** Each column becomes, in this order of precedence: what the
  `--map` file says; else the database library's own field map (the key, a
  reserved name, or a parameter name); else a parameter named like the column.
  The same recognition as for symbol parameters then applies by name:
  `Manufacturer`/`Manufacturer Part Number` (numbered variants are
  alternates), `Supplier n`/`Supplier Part Number n`/`Supplier Order Qty n`,
  `Datasheet`, `HelpURL`, `ComponentLinkNURL`, and the class template's
  slots (a measurement only when the value parses with a unit).
- **Reserved names.** `[Library Ref]` and `[Library Path]` name the symbol,
  `[Footprint Ref]`/`[Footprint Path]` the footprint, `[Footprint Ref n]`/
  `[Footprint Path n]` (n = 2, 3, ...) the alternate footprints in order,
  `[Description]` the description. Simulation fields (`[Sim ...]`) are
  reported once per table (`table.sim_skipped`, info) and not imported.
- **Paths.** A library path is looked up as given, then relative to the
  `.DbLib`, then by file name (without case; `\` and `/` both separate) in the
  `--search-path` folders and the `.DbLib`'s own search path (relative to it,
  with sub-folders when it says so). Paths from another machine therefore
  resolve by their file name. Each library is read once, however many rows
  name it, and only the symbols and footprints rows name are written.
- **Skipped rows.** A row whose symbol, or any footprint it names, cannot be
  found is skipped and reported as an `error` (`row.symbol_missing`,
  `row.symbol_library_missing`, `row.footprint_missing`,
  `row.footprint_library_missing`), so the run exits 2. A row that names no
  footprint gets a part without one. Parts are `released`.
- **The database.** `--db` overrides the stored one: a folder of per-table
  `<table>.csv` files, an Access file, or a connection string naming one.
  Otherwise the stored path is looked up like a library path. On Linux an
  Access file (`.mdb`, `.accdb`) is read by running `mdb-tables` and
  `mdb-export` (mdbtools, GPL) as separate processes; they are never linked
  or bundled, and a missing mdbtools gives an error with the install hint.
  Direct connections through the Access driver (Windows) are not
  implemented: use `--db <folder>` there. Other database kinds: export the
  tables to CSV.
- **The mapping file.** `--map map.toml` has one table per database table
  (`[tables."Resistors"]`), with `"<column>" = "<target>"` entries; a target
  is `[Key]`, `[Skip]`, a reserved name, or a parameter name (a column named
  `Maker` mapped to `Manufacturer` is read as the manufacturer). The mapping
  every table actually used is written as `import-mapping.toml` beside the
  report in the same format; passing it back reproduces the run.

## Report codes

Every approximation or loss has a stable code, `<area>.<what>`; a code never
changes meaning. The single-file `--json` summary groups items by message and
lists all of them; the library report (`import-report.json`) keeps each item
with its code, severity and source. Severities are the library converter
contract's: `info` (nothing lost), `approximated` (written, approximately),
`dropped` (written, part left out), `error` (item not written).

| Code | Severity | Meaning |
|---|---|---|
| `symbol.pin_ieee_unmapped` | dropped | A pin's IEEE symbol code with no Signex shape (detail `ieee_code` and `slot`: `inside`, `outside`, `inner_edge`, `outer_edge`) |
| `symbol.bezier_polygon` | approximated | A library symbol bezier that is not a run of cubic curves (4, 7, 10, ... points) imported as its control polygon |
| `symbol.rounded_rect_radius` | approximated | A library symbol rounded rectangle with different x and y corner radii takes the smaller |
| `symbol.display_mode_dropped` | dropped | A pin or graphic of a display mode beyond the two Signex bodies |
| `symbol.pin_decoration_merged` | approximated | A pin with an inner and an outer edge symbol keeps both; placement shows the outer |
| `symbol.text_sanitized` | approximated | Control characters or a triple apostrophe in a pin text replaced |
| `footprint.text_sanitized` | approximated | Control characters or a triple apostrophe in a pad number replaced |
| `part.pin_map_unread` | error | A footprint model has a pin map this reader cannot decode; part skipped |
| `sch.rounded_rect_square` | approximated | Sheet rounded rectangle imported with square corners |
| `sch.bezier_polygon` | approximated | Sheet bezier imported as its control polygon |
| `sch.image_dropped` | dropped | Embedded image not imported |
| `sch.record_dropped` | dropped | Other schematic record type not imported (detail `record`) |
| `footprint.courtyard_text_dropped` | dropped | Text on a courtyard layer not imported |
| `footprint.graphic_layer_unmapped` | dropped | Footprint graphic on a layer the format has no list for (mechanical, assembly beyond fab, ...) dropped; tracked as signex #3133 (detail `layer`) |
| `footprint.copper_track_dropped` | dropped | Copper track inside a library footprint not imported |
| `footprint.fill_dropped` | dropped | Fill inside a library footprint not imported |
| `footprint.region_dropped` | dropped | Region (custom pad shape) inside a library footprint not imported |
| `footprint.courtyard_open_bbox` | approximated | Open courtyard outline replaced by its bounding box |
| `footprint.pad_octagon_custom` | approximated | Octagonal library pad written as a custom outline with regular-octagon cuts (the cut is not read from the file) |
| `pad.slot_not_placeable` | approximated | A footprint has a slotted drill. It is written exactly (slot length kept), but Signex cannot place it on a board until slotted drills are supported (signex #3128). Once per footprint. |
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
| `footprint.name_collision` | approximated | A footprint name is already defined; parts bind to the first |
| `model.body_without_model` | info | A body with no 3D model is not imported |
| `model.skipped` | info | A model skipped by `--models skip` |
| `model.extra_body` | error | A footprint holds one model; a further body's model is not imported |
| `model.missing` | error | The body's model is not in the library's model table |
| `model.external` | error | The model is not embedded in the library |
| `model.unreadable` | error | The model's stream is missing or does not inflate |
| `model.not_step` | error | The model is not a STEP file |
| `model.placement_approximated` | approximated | A 2D rotation is added to the z turn, or a bottom-side projection is not applied |
| `library.crossref_mismatch` | info | An integrated library's cross-reference disagrees with its schematic library |
| `library.kept_edited` | info | A file this converter wrote that left the source was edited since; kept |
| `library.kept_referenced` | info | A file that left the source is kept because a row you added still uses it |
| `table.disabled` | info | A table disabled in the database library is skipped |
| `table.missing` | error | An enabled table is not in the database |
| `table.no_key` | error | A table has no key field; its rows are skipped |
| `table.sim_skipped` | info | Simulation fields of a table are not imported |
| `row.key_missing` | error | A row has no key value; skipped |
| `row.symbol_missing` | error | A row names no symbol, or the library has no such symbol; skipped |
| `row.symbol_library_missing` | error | A row's symbol library was not found or could not be read; skipped |
| `row.footprint_missing` | error | A footprint a row names is not in its library; skipped |
| `row.footprint_library_missing` | error | A footprint library a row names was not found or could not be read; skipped |
| `table.reserved_unknown` | info | A bracketed target this converter does not know is kept as a parameter of that name |
| `table.lossy_text` | info | A column held bytes that are not UTF-8; they were replaced |
| `library.search_truncated` | info | A search folder holds more files than the listing limit |
| `row.symbol_library_unreadable` | error | A row's symbol library exists but could not be read; skipped |
| `row.footprint_library_unreadable` | error | A row's footprint library exists but could not be read; skipped |
| `symbol.unreadable` | error | A component of a schematic library could not be read; not written |
| `part.symbol_unreadable` | error | The part of a component that could not be read; not written |
| `library.input_unreadable` | error | An input file could not be read (library runs) |

81 codes. The catalogue lives in `crates/signex-altium-convert/src/codes.rs`.
