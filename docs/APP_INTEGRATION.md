# Signex app integration

Signex's **File › Import › Altium Project…** runs this binary as a
separate process. Nothing is linked into Signex; the two share only the
files on disk and this contract.

## Discovery

1. The path in the `SIGNEX_ALTIUM_IMPORT` environment variable, if set.
2. Otherwise, `signex-altium-import` on `PATH`.

## Invocation

```text
signex-altium-import <input> --output <dir> --json
```

Signex never passes `--force`. If outputs already exist, the run fails
and Signex shows that error, so an import can never replace a user's
files.

## Result

One JSON object on stdout, whether the run succeeds or fails, and exit
status 0 on success, non-zero on failure.

```json
{
  "ok": true,
  "error": null,
  "project": "/abs/out/Board.snxprj",
  "files": ["/abs/out/Main.snxsch", "/abs/out/Board.snxpcb", "/abs/out/Board.snxprj"],
  "warnings": [{ "message": "copper arc imported as straight track segments", "count": 12 }],
  "track_ends": null
}
```

`project` is `null` when the input is a single document or library;
Signex then opens the first entry of `files` (a `.snxsch`, `.snxpcb`,
`.snxsym` or `.snxfpt`). Fields may be added; existing fields
keep their meaning.

`track_ends` is a self-check of a single converted board (`.PcbDoc`
input), `null` otherwise: `{ "total", "on_own_net_pad",
"on_other_net_pad" }`, counting track ends that land on a netted pad with
copper on the track's layer. Ends on another net's pad should be about
zero; many mean pads were placed wrong.

## Library converter contract

Besides the single-file import above, the binary is a Signex *library
converter*. The authoritative contract is
`docs/LIBRARY_CONVERTER_CONTRACT.md` in the
[Signex repository](https://github.com/alplabai/signex) (version `1.0`), with
a JSON Schema beside it; this section only says how this binary meets it. The
single-file contract above is unchanged.

```text
signex-altium-import describe --json
signex-altium-import library <input>... --output <dir>
    [--name <name>] [--library-id <uuid>] [--db <path>] [--search-path <dir>]...
    [--map <mapping.toml>] [--models include|skip] [--models-dir <dir>]
    [--dry-run] [--update] [--json] [--progress]
```

- `describe` lists the inputs: `schlib`, `pcblib`, `intlib` (kind `library`)
  and `dblib` (kind `database`).
- `library` runs read, convert and write phases. `--progress` streams NDJSON
  events, flushed per line, with a heartbeat at least every 10 s; the
  `summary` event is always last and implies `--json`.
- Every run that read its inputs writes `import-report.json` and
  `import-report.md` into `--output`; codes are catalogued in
  `docs/CONVERSION.md` ("Report codes").
- Exit codes: 0 `ok`, 2 `partial` (something was dropped or not written), 1
  `failed`. An unknown option, a bad value or an unusable `--output` exits 1
  with a failed summary on stdout, even without `--json`.

Status: reading `.SchLib` and `.PcbLib`, option validation, progress, the
report and the summary are in place, and `--dry-run` works end to end. Writing
the `.snxlib` itself is not: a run without `--dry-run` writes the report and
fails with `library output not implemented yet`. `.IntLib`, `.DbLib` and
folder inputs are advertised by `describe` but refused until their readers
land. `--help` and `--version` print no summary. A failed run removes the
library files it wrote, keeps `import-report.*` once the source was read, and
leaves a library that existed before `--update` as it was. In a dry run the summary's `files` is empty and `counts` are zero.
