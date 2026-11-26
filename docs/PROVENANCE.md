# Provenance

This repository is FSL-1.1-ALv2 (each release becomes Apache-2.0 two years
after it ships; releases published before the licence change stay
Apache-2.0). It must stay compatible with Signex's
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
- **A private corpus** of company libraries (integrated, schematic, footprint
  and database libraries). Some layouts in `FORMAT_NOTES.md` were observed
  only there and are marked "observed in a private corpus file": the
  integrated-library container, the database-library file, the integrated
  libraries' empty model table, the stale DIFAT entries the container repair
  frees, and the `0x8E` value escape (also seen in a public sample). They are
  described generically; no file, name, part number or value from the corpus
  is in this repository.

## Sources not used

- **`wavenumber-eng/altium_monkey` and `altium_cruncher`** (AGPL-3.0). Only
  their README feature lists were read, to scope this tool. Their source,
  docs and tests were not read, and nothing here derives from them.
- **Any GPL-licensed EDA tool's Altium importer.** None was placed in
  context or consulted.
- **A reference converter, compared black-box only.** To check output, a
  third-party Altium reader was installed in an isolated environment outside
  this repository and run on the public samples and the private corpus; only
  its `--help` text and its output were read. Its source was never read, it
  is not a dependency, and it is not run in this repository or its CI. Where
  this reader follows an observation from that comparison, `FORMAT_NOTES.md`
  states the evidence in the files themselves.
- **mdbtools** (GPL), for Access database libraries: run only as separate
  programs (`mdb-tables`, `mdb-export`) when the user has installed them.
  It is never linked, vendored or shipped, and none of its source was read.

## Method

The reader was written with LLM assistance. No third-party importer
source was put in the model's context. Each record layout was derived by
dumping the sample streams (`cargo run -p altium-format --example dump`)
and matching fields against values Altium itself reports in `simple.csv`.
The tests in `crates/altium-format/tests/` pin those matches down.
