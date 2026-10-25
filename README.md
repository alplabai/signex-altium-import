# signex-altium-import

One-way converter from Altium Designer files to Signex native files.

| Altium | Signex | Status |
|---|---|---|
| `.PrjPcb` | `.snxprj` + one `.snxsch` per sheet + `.snxpcb` | supported |
| `.SchDoc` | `.snxsch` (symbols embedded) | supported |
| `.PcbDoc` | `.snxpcb` | supported |
| `.SchLib` / `.PcbLib` | — | read and listed; library export not yet implemented |

**Apache-2.0**, like [Signex](https://github.com/alplabai/signex). The reader
is a clean-room implementation; see [docs/PROVENANCE.md](docs/PROVENANCE.md).

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

## Install

From source:

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
  signex-types/           vendored from alplabai/signex (scripts/sync-signex-types.sh)
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
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo run -p altium-format --example dump -- file.SchDoc [stream] [sch|pcbbin|pcbtext|hex]
```

## License

Apache-2.0. See `LICENSE` and `NOTICE`.
