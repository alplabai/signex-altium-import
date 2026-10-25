# Altium binary format notes

What `altium-format` relies on, as verified against the files in
`tests/samples/`. Byte offsets are within a record's payload, little-endian.

## Container

Binary Altium files are OLE2 compound documents (Compound File Binary).
Each document type keeps its records in named streams.

## Schematic (`.SchDoc`, `.SchLib`)

**Framing.** A stream is a run of frames. Each frame starts with a `u32`:
the low 24 bits give the payload length and the high byte gives its kind.
Kind 0 is text: `|KEY=VALUE|KEY=VALUE…`, NUL-terminated, Windows-1252,
with a `%UTF8%KEY` entry overriding `KEY`. Kind 1 is a packed binary
record (library pins).

**Documents.** Stream `FileHeader`. Frame 0 is the header. The other
frames are records numbered from 0, and a child names its parent with
`OwnerIndex`. `RECORD=` is the type (see `sch::kind`).

**Libraries.** `FileHeader` lists `CompCount` and `LibRef{i}`. Each
symbol is a storage named after its library reference (at most 31
characters, `/` not allowed). Longer names are mapped in `SectionKeys`
through `LibRef{i}` and `SectionKey{i}`. The symbol's `Data` stream starts
with its component record, and the children have no `OwnerIndex`.

**Units.** Coordinates are in 10-mil units. `KEY_FRAC` adds 1/100000 of a
unit. Y points up.

**Binary pin** (kind 1, `RECORD` 2):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | record id (2) |
| 4 | 1 | reserved |
| 5 | 2 | owner part id (i16) |
| 7 | 1 | display mode |
| 8 | 4 | IEEE symbols: inner edge, outer edge, inside, outside |
| 12 | 1+n | description (length-prefixed) |
| … | 1 | formal type (1) |
| … | 1 | electrical type (0 in, 1 I/O, 2 out, 3 OC, 4 passive, 5 HiZ, 6 OE, 7 power) |
| … | 1 | conglomerate (below) |
| … | 2 | length (i16, 10-mil) |
| … | 2+2 | x, y (i16, 10-mil): the body end of the pin |
| … | 4 | colour |
| … | 1+n | name |
| … | 1+n | designator |

The conglomerate byte, which is the same as text `PinConglomerate`:

| Bits | Meaning |
|---|---|
| 0–1 | quarter turns CCW from +X, from the body end toward the electrical end |
| 2 | hidden (no sample exercises this bit) |
| 3 | name shown |
| 4 | designator shown |

The sample notes that ship with the samples confirm the bit-3/bit-4
assignment: clearing bit 4 hides the designator and clearing bit 3 hides
the name.

`PinFrac` (per symbol) holds fractions, `[u8 tag][len-prefixed pin
index][u32 len][zlib]`. Inflated, it gives three `i32` values (x, y,
length) in 1/100000 units.

## PCB (`.PcbDoc`, `.PcbLib`)

**Units.** Binary coordinates are `i32` in 1/10000 mil. Text records carry
a unit suffix (`1574.8031mil`). Y points up. Coordinates are absolute;
`Board6` `ORIGINX/Y` gives the user origin.

**Text streams** (`Board6`, `Nets6`, `Components6`, `Polygons6`, …) are
runs of `[u32 len][params]`.

**Primitive streams** (`Tracks6`, `Arcs6`, `Pads6`, `Vias6`, `Texts6`,
`Fills6`, `Regions6`) are runs of `[u8 type][u32 len][block]…`. Most types
have one block. Texts have two blocks (main, string) and pads have six
(name, three reserved, main, size-and-shape).

The common header of every main block:

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | layer |
| 1 | 1 | flags (bit 2 = *unlocked*) |
| 2 | 1 | flags (bit 1 = keepout) |
| 3 | 2 | net index into `Nets6`, `0xFFFF` = none |
| 5 | 2 | polygon index, `0xFFFF` = none |
| 7 | 2 | component index into `Components6`, `0xFFFF` = free |
| 9 | 4 | reserved |

The type-specific fields that follow, from offset 13:

| Type | Fields |
|---|---|
| 1 arc | center x, y; radius; start, end (f64 degrees); width |
| 3 via | x, y; diameter; hole; start layer (u8); end layer (u8) |
| 4 track | x1, y1, x2, y2; width |
| 6 fill | x1, y1, x2, y2; rotation (f64) |
| 2 pad (main) | x, y; size top/mid/bottom (x, y each); hole; shape top/mid/bottom (u8: 1 round, 2 rect, 3 octagonal); rotation (f64); plated (u8) |
| 5 text | x, y; height; stroke font (u16); rotation (f64); mirrored (u8); stroke width; is comment (u8); is designator (u8); …; wide-string index (u32 at 115) |
| 11 region | 5 reserved; `[u32 len][params]`; `u32` count; count × (f64 x, f64 y) |

The pad size-and-shape block holds 29 mid-layer sizes (x then y), 29
mid-layer shapes, one reserved byte, hole shape (0 round, 1 square, 2
slot), slot length, hole rotation (f64), 32 × (x, y) offsets, a
"per-layer shapes present" byte, 32 per-layer shapes (9 = rounded
rectangle) and 32 corner radii (percent of half the short side), with
index 0 for the top layer.

`WideStrings6` is a run of `[u32 index][u32 byte len][UTF-16LE]`.

**Layers:**

| Id | Layer |
|---|---|
| 1 | top |
| 2–31 | mid 1–30 |
| 32 | bottom |
| 33 | top overlay |
| 34 | bottom overlay |
| 35 | top paste |
| 36 | bottom paste |
| 37 | top solder |
| 38 | bottom solder |
| 39–54 | internal plane 1–16 |
| 55 | drill guide |
| 56 | keep-out |
| 57–72 | mechanical 1–16 |
| 73 | drill drawing |
| 74 | multi-layer |

**Library footprints.** Each footprint is a storage whose `Data` stream
starts with `[u32 len][length-prefixed name]` and is followed by
primitive records of any type.

## Project (`.PrjPcb`)

INI text. Each `[DocumentN]` section names one document in
`DocumentPath`. `[ParameterN]` sections hold `Name` / `Value` pairs.
