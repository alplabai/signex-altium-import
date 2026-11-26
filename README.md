# signex-altium-import

One-way converter from Altium Designer files to Signex native files.

| Altium | Signex | Status |
|---|---|---|
| `.PrjPcb` | `.snxprj`, one `.snxsch` per sheet, `.snxpcb`, and a `.snxsym` / `.snxfpt` per listed library | supported |
| `.SchDoc` | `.snxsch` (symbols embedded) | supported |
| `.PcbDoc` | `.snxpcb` | supported |
| `.SchLib` | `.snxsym` (every symbol in one file) | supported |
| `.PcbLib` | `.snxfpt` (every footprint in one file) | supported |
| `.SchLib` + `.PcbLib` (`library`) | one Signex library directory | supported |

Source-available under **FSL-1.1-ALv2**, like
[Signex](https://github.com/alplabai/signex); see [License](#license). The
reader is a clean-room implementation; see [docs/PROVENANCE.md](docs/PROVENANCE.md).

## Use

```bash
signex-altium-import path/to/Board.PrjPcb
```

This writes `Board.snxprj` and its sheets and board next to the project
(`-o DIR` to put them elsewhere). Open the `.snxprj` in Signex. Existing
files are never replaced unless you pass `--force`.

Signex can also run the importer for you: **File › Import › Altium
Project…** finds `signex-altium-import` on your `PATH` (or at the path in
`SIGNEX_ALTIUM_IMPORT`) and opens the result.

Every run ends with a list of what was approximated or skipped, for
example copper arcs flattened to segments or 3D bodies left out. Read it
before trusting an imported board for fabrication.

```text
--ports auto|hierarchical|global   how Altium ports become Signex labels
--json                             machine-readable summary (used by Signex)
```

### Library converter

Signex also drives the binary as a library converter (contract in
[docs/APP_INTEGRATION.md](docs/APP_INTEGRATION.md)):

```bash
signex-altium-import describe --json
signex-altium-import library Passives.SchLib Passives.PcbLib --output out/Passives --dry-run --progress
```

`library` writes the library (a `.snxlib`, its symbol and footprint files)
plus `import-report.json` and `import-report.md` into `--output`, and exits 0
(ok), 2 (partial) or 1 (failed). `--dry-run` writes only the report.

## Install

Download the archive for your platform from
[Releases](https://github.com/alplabai/signex-altium-import/releases) and put
`signex-altium-import` on your `PATH`. Or build from source:

```bash
cargo install --git https://github.com/alplabai/signex-altium-import signex-altium-import
```

## Layout

```
crates/
  altium-format/          reader: OLE container, record framing, typed
                          SchDoc / SchLib / PcbDoc / PcbLib / PrjPcb models
  signex-altium-convert/  mapping onto signex-types, with a loss report
  cli/                    the signex-altium-import binary
  signex-types/           vendored from alplabai/signex
  signex-expr/            (scripts/sync-signex-crates.sh; the last three
  signex-sketch/           trimmed to their file formats)
  signex-library/
tests/samples/            Altium sample files (see tests/samples/NOTICE.md)
docs/                     format notes, conversion rules, app contract
```

`altium-format` knows nothing about Signex and can be used on its own:

```rust
let doc = altium_format::sch::SchDoc::open("Main.SchDoc")?;
for c in &doc.components {
    println!("{} {}", c.designator.as_ref().map_or("?", |d| &d.text), c.lib_reference);
}
```

## Develop

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude signex-library --exclude signex-sketch --exclude signex-expr --all-targets -- -D warnings
cargo test --workspace --exclude signex-library --exclude signex-sketch --exclude signex-expr
cargo run -p altium-format --example dump -- file.SchDoc [stream] [sch|pcbbin|pcbtext|hex]
```

## License

signex-altium-import is source-available under the
[Functional Source License 1.1, Apache 2.0 Future License (FSL-1.1-ALv2)](LICENSE):

- **Allowed:** any use, including companies designing and selling their own
  products with it.
- **Not allowed:** a Competing Use, which means offering a rival product or
  service built from this code.
- **Later:** each release becomes Apache-2.0 two years after it ships.

Releases published before the licence change stay Apache-2.0.

The sample files in `tests/samples/` are third-party files under Apache-2.0;
see `NOTICE` and `tests/samples/NOTICE.md`.

Copyright 2026 [Alp Lab AB](https://github.com/alplabai)
