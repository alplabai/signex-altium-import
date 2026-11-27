# Altium binary format notes

What `altium-format` relies on, as verified against the files in
`tests/samples/`. Byte offsets are within a record's payload, little-endian.

## Container

Binary Altium files are OLE2 compound documents (Compound File Binary).
Each document type keeps its records in named streams.

Observed in a private corpus file: a header DIFAT with entries past the
declared FAT sector count that do not hold the free marker. Such a file is
read after setting those entries free (`library.difat_repaired`).

## Schematic (`.SchDoc`, `.SchLib`)

**Framing.** A stream is a run of frames. Each frame starts with a `u32`:
the low 24 bits give the payload length and the high byte gives its kind.
Kind 0 is text: `|KEY=VALUE|KEY=VALUE…`, NUL-terminated, Windows-1252,
with a `%UTF8%KEY` entry overriding `KEY`. Kind 1 is a packed binary
record (library pins).

**`|` inside a value.** A raw `|` would end the field, so a value stores it
as byte `0x8E` in the ANSI form and as `¦` (U+00A6) in the `%UTF8%` form.
Observed in `tests/samples/simple.SchDoc`: pin records carry
`SwapIDPart=<8E>&<8E>` beside `%UTF8%SwapIDPart=¦&¦`, the same `|&|` that
library pads hold length-prefixed (`Simple.PcbLib`, block after the pad
name). Observed in a private corpus file: parameter values whose ANSI form
holds `0x8E` where a black-box reference read shows `|`. The reader decodes
`0x8E` in a value as `|`, and `¦` in a `%UTF8%` value as `|` only when the
ANSI form has a `0x8E` and no `0xA6` (a real `¦`). Keys, length-prefixed
strings and other text are not touched.

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

### Integrated library (`.IntLib`)

Observed in a private corpus file (two integrated libraries; no sample is
committed). The compound file holds, at its root:

- `SchLib/<n>.schlib` and `PCBLib/<n>.pcblib`: one embedded library each, in
  numbered streams (one of each in the observed files). The stream is a type
  byte `0x02` followed by a zlib stream (`78 xx`); the inflated bytes are
  themselves a compound file, a complete `.SchLib` or `.PcbLib`, read by the
  existing readers.
- `Version.Txt`: five bytes.
- `LibCrossRef.Txt` and `Parameters   .bin` (three spaces in the name): same
  type byte and zlib framing. The cross-reference opens with a `u32` entry
  count equal to the number of schematic components, then per-entry
  length-prefixed strings (component name, a path inside the file, a
  description, a source path, the footprint name). It is read only as a
  cross-check (count and names), never for bindings. `Parameters` holds
  per-component parameter text and is not read.
- `CKT/<n>.ckt` simulation models in one of the files: not read.

In the observed integrated libraries the embedded footprint library's model
table (`Library/Models/Data`, `Library/ModelsNoEmbed/Data`) is empty while
its component bodies name models by id with no embed flag: the 3D models are
stored nowhere in the file. The root also holds a storage of simulation
model streams (`MDL/<n>.mdl`), one with a type byte `0x00`, one zlib
(type `0x02`); they are not read.

A stream that is empty, has another type byte, does not inflate, or inflates
to something that is not a compound file is an error, as is a file with no
embedded library at all.

### Component bodies and the model table (`.PcbLib`)

Observed in `tests/samples/Simple.PcbLib`.

- A footprint's primitive stream holds record type 12, a component body: the
  common 13-byte header, five bytes, then a length-prefixed text block of
  `KEY=value` pairs. The keys used: `MODELID` (a GUID in braces),
  `MODEL.NAME`, `MODEL.EMBED`, `MODEL.2D.X` / `MODEL.2D.Y` (lengths in mil),
  `MODEL.2D.ROTATION`, `MODEL.3D.ROTX` / `ROTY` / `ROTZ` (degrees),
  `MODEL.3D.DZ` (length), `BODYPROJECTION`, `OVERALLHEIGHT`. Outline geometry
  follows the text and is not read.
- `Library/Models/Data` is a run of length-prefixed text records, one per
  model: `ID` (the GUID a body names), `NAME`, `EMBED`, `ROTX/Y/Z`, `DZ`,
  `CHECKSUM`. Record *i* belongs to stream `Library/Models/<i>`, a bare zlib
  stream (no type byte) holding the model file; in the sample, a STEP file.

### Database library (`.DbLib`)

Observed in a private corpus file. An INI text file in the local code page
(no byte-order mark). `[DatabaseLinks]` holds `ConnectionString`,
`LibraryDatabaseType`, `LibraryDatabasePath`, `DatabasePathRelative`,
`LibrarySearchPath` (folders joined by `;`, relative to the file),
`SearchSubDirectories`, quoting keys and others. Each table is a `[TableN]`
with `TableName`, `SchemaName`, `Enabled` (`True`/`False` or `1`/`0`),
`UserWhere` and `UserWhereText`. Each column mapping is a `[FieldMapN]` with
one `Options=` line of `key=value` pairs joined by `|`: `FieldName`
(`<table>.<column>`), `TableNameOnly`, `FieldNameOnly`, `FieldType`,
`ParameterName` (a parameter, or a reserved name in brackets such as
`[Library Ref]`, `[Footprint Path 2]`), and the add, update and remove modes.
`FieldType=0` marks the key field: exactly one per enabled table. Columns of
a table without a field map exist in the database and are parameters of their
own name. Field maps belong to a table by `TableNameOnly`. Disabled tables
may have no field maps.

### Container time

A library's compound-file root entry may record a modified time; a zero time
reads as 1601 and counts as none (`SchLib::modified`, `PcbLib::modified`).
The library converter stamps its output with the newest of them.

### Implementations (footprint links)

A component's footprint and other model links are records 44 to 48 below the
component record:

- `RECORD=44` is the list, owned by the component.
- `RECORD=45` is one model, owned by the list: `ModelName`, `ModelType`
  (`PCBLIB` for a footprint; other kinds name other models, for example a
  simulation), `IsCurrent=T` on the one in use, plus `DatafileCount`,
  `ModelDatafileEntity0` and `ModelDatafileKind0`. Observed in
  `tests/samples/simple.SchDoc` (`ModelName=R0603L`, `ModelType=PCBLIB`,
  `IsCurrent=T`).
- `RECORD=46` and `RECORD=48` follow each model, owned by it, and carry no
  keys in the samples (`simple.SchDoc`). Pin maps would live below record 46;
  **no sample holds one, so they are not decoded**: `Implementation::pin_map`
  stays empty. A record 46 with keys, or any record 47, sets
  `Implementation::pin_map_unread`, and the converter skips that part
  (`part.pin_map_unread`) instead of binding it 1:1. Decoding needs a sample
  with a non-trivial pin map.
- The `SchLib` samples hold a list (44) with no models at all, so the library
  samples bind no footprints; the binding tests build components in memory.

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

**`LayerKindMapping`** assigns roles to mechanical layers. It holds a
length-prefixed UTF-16 version string, a `u32` checksum and a `u32` count,
then that many `(u32 layer id, u32 kind)` pairs. The samples confirm these
kinds, by matching them against layer names:

| Kind | Role |
|---|---|
| 1 | assembly top |
| 2 | assembly bottom |
| 11 | courtyard top |
| 12 | courtyard bottom |
| 26 | 3D body top |
| 27 | 3D body bottom |

**Library footprints.** Each footprint is a storage whose `Data` stream
starts with `[u32 len][length-prefixed name]` and is followed by
primitive records of any type. The name block holds exactly the string:
its length is one more than the string's length byte. Observed in
`tests/samples/Simple.PcbLib`:

- `Library/Data` follows its header text record with a `u32` count (13) and
  that many `[u32 len][length-prefixed name]` entries, the footprints in
  library order; storage names are cut to 31 characters
  (`SQFP50P800X800X300_HS-33T300X30` holds `…T300X300`), with the cut name
  also listed in `SectionKeys`.
- The root also holds storage `FileVersionInfo` (as do most footprint
  libraries of a private corpus), whose `Data` stream is one
  `[u32 len]|COUNT=n|VER0=…` text record. A record of 125 bytes would look
  like a name block (`|` is 124), so the reader takes as footprints only the
  storages whose name block names an entry of the directory, and reports
  the others (`library.storage_skipped`). Without a directory, a name block
  must hold exactly one length-prefixed string. `Library` is skipped by name.

## Project (`.PrjPcb`)

INI text. Each `[DocumentN]` section names one document in
`DocumentPath`. `[ParameterN]` sections hold `Name` / `Value` pairs.
