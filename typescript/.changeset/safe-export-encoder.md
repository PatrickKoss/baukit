---
'@baukit/data-contracts': minor
---

Add the `/export` entry point, also re-exported from the package root. `encodeCsv` writes RFC 4180 CSV with CRLF record separators and neutralizes spreadsheet formulas by default: a text cell that starts with `=`, `+`, `-`, `@`, tab, or carriage return, or with `=`, `+`, `-`, or `@` after leading spaces, tabs, CR, or LF, gets a leading apostrophe. Pass `neutralizeFormulas: false` to turn this off, and `byteOrderMark: true` to start the output with U+FEFF. Finite `number` cells and `{ numeric }` cells (built with `csvNumeric`) must match the JSON number grammar and are written without neutralization, so `-5` stays a number. Invalid numeric cells, unpaired surrogates, and unsupported runtime values throw `CsvEncodeError` with a code, row index, and column index, never the cell value. `ShareOutcome` and `SHARE_OUTCOMES` name the share or save results `shared`, `saved`, `cancelled`, `unavailable`, and `failed`; no Expo implementation ships yet. `baukit_core::export::encode_csv` passes the same vectors in `fixtures/export-csv/csv-encoding-v1.json`.

No breaking changes.
