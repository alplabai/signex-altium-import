Synthetic database fixture: no real part data.

- `Synthetic.DbLib` and `csv/*.csv` were written for this repository and are
  released under CC0-1.0.
- The schematic and footprint libraries the rows name are the public samples
  of `tests/samples/` (Apache-2.0, see `tests/samples/NOTICE.md`); the test
  copies them into a temporary `Libs/` folder as `Passives.SchLib`,
  `Misc.SchLib`, `Footprints1.PcbLib` and `Footprints2.PcbLib`.
- A generated Access file is not committed: Access files cannot be written
  without Access or its drivers. The same rows are read through the CSV path,
  and the Access path is exercised on the private corpus (pool only).
