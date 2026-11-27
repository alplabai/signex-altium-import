`library-converter-v1.json` is a verbatim copy of
`docs/schemas/library-converter-v1.json` from the Signex repository
(alplabai/signex), commit 9a3fdc1cb (unchanged at signex trunk 68c548bef). It is the machine-checkable part of the
library converter contract (`docs/LIBRARY_CONVERTER_CONTRACT.md` there). Do not
edit it here; refresh it from Signex when the contract changes.

`legacy/*.json` are the single-file `--json` outputs of the importer before
the library contract work, captured from `tests/samples`, with the output
directory written as `<OUT>` and the samples directory as `<SAMPLES>`. They
pin the legacy output byte for byte.
