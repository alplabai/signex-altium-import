# Provenance

This repository is Apache-2.0 and must stay compatible with Signex's
licence policy (`CONTRIBUTING.md` § "License compliance for contributions"
in alplabai/signex). This file records what the reader was built from.

## Sources used

- **Sample files and their notes:** from `pluots/altium` (Apache-2.0).
  These are the `.SchDoc`, `.SchLib`, `.PcbDoc`, `.PcbLib` and `.PrjPcb`
  files in `tests/samples/`, the Altium-exported `simple.csv`, and the
  annotated hex dumps that ship beside them. Every layout in
  `FORMAT_NOTES.md` was checked against these bytes.
- **The `pluots/altium` source** (Apache-2.0), read for the binary
  schematic pin layout. Its show-name / show-designator bit masks are
  swapped relative to what its own sample notes show; this reader follows
  the sample data.
- **The Compound File Binary container:** read through the `cfb` crate
  (MIT).

## Sources not used

- **`wavenumber-eng/altium_monkey` and `altium_cruncher`** (AGPL-3.0). Only
  their README feature lists were read, to scope this tool. Their source,
  docs and tests were not read, and nothing here derives from them.
- **Any GPL-licensed EDA tool's Altium importer.** None was placed in
  context or consulted.

## Method

The reader was written with LLM assistance. No third-party importer
source was put in the model's context. Each record layout was derived by
dumping the sample streams (`cargo run -p altium-format --example dump`)
and matching fields against values Altium itself reports in `simple.csv`.
The tests in `crates/altium-format/tests/` pin those matches down.
