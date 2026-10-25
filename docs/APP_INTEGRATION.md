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
  "warnings": [{ "message": "copper arc imported as straight track segments", "count": 12 }]
}
```

`project` is `null` when the input is a single document; Signex then
opens the first entry of `files`. Fields may be added; existing fields
keep their meaning.
