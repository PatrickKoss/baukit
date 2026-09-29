---
'@baukit/data-contracts': patch
---

Add two `encodeCsv` options that keep a null cell apart from empty text. `quoteAllCells: true` quotes every text and numeric cell and leaves null cells unquoted. `nullMarker` writes a marker such as `\N` unquoted for a null cell and quotes a text cell with the same content; an empty marker or one holding a double quote, comma, CR, or LF throws a `RangeError`. `baukit_core::export::CsvOptions` gains `with_all_cells_quoted` and `with_null_marker`, and both pass five new shared vectors.

Default output is unchanged. No breaking changes.
