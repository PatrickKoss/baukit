# Safe export encoder evidence

Plan item 20, "Add a safe export encoder and share outcome".

## Source revisions

- Baukit baseline `8b2d11f` (after wave 1 merged).
- Eigenruhe `f74cebb`. The plan surveyed `e44ff88`; the files below were re-read at `f74cebb`.
- Hebkit `841bf5d`, Tiefgang `2d37a06`, Redemut `a782538`, Runtime Analyzer `d47bfd5`, and Solo
  Leveling System `3461eaf`, the survey revisions.

## Observed failure

Three TypeScript products write CSV by hand, and none neutralizes formula prefixes:

- Eigenruhe `mobile/src/features/data-transfer/csv.ts:73` quotes a cell only for `"`, `,`, CR, or
  LF. The session export includes user notes and custom tag labels, and it writes numbers with
  `String(...)` into the same text path.
- Hebkit `mobile/src/integrations/files/csv.ts:65-82` does the same, plus quoting for a leading `#`
  and for its `\N` null marker. This file is a round-trip codec: `parseRows` (`:84`) reads the
  export back, so any change to written values must be mirrored in the parser.
- Tiefgang `mobile/src/integrations/files/csv.ts:21-50` quotes the same four characters and writes
  every table, including project names, notes, intentions, and negative `delta_credits` values.

The plan also listed Redemut's account export. At `a782538` that export is JSON, not CSV:
`web/src/account-screen.tsx:641-684` builds a JSON object and downloads it, and
`mobile/src/account.tsx:590-622` passes the same JSON to React Native `Share.share`. Redemut has no
CSV writer and no formula injection. Its share call is still evidence for the outcome comparison
below.

On the Rust side, Runtime Analyzer `backend/crates/finops-api/src/routes/audit.rs:82-97` guards.
It trims leading space, tab, CR, and LF, prefixes an apostrophe when `=`, `+`, `-`, or `@` follows,
and quotes every cell. It uses LF line endings, and a value that starts with a tab or CR followed by
plain text is not neutralized. Solo Leveling System
`backend/crates/sl-services/src/orgs.rs:1499-1526` does not neutralize, quotes only for `,`, `"`,
and LF, so a CR inside a value is written unquoted, and uses LF line endings. Its report payload
holds computed aggregates, so the injection risk there depends on whether a string value can ever
come from user input.

## Baukit owner

`@baukit/data-contracts` owns the `/export` entry point, also re-exported from the root.
`baukit-core` owns the `export` module. Both are dependency-free apart from `thiserror` in Rust,
which `baukit-core` already depends on, so the default dependency set is unchanged and no feature
flag was needed. Products own column choice, header labels, value formatting, file names, media
types, and the share or save step.

## Decisions

**Formula prefixes.** A text cell is neutralized when its first character is `=`, `+`, `-`, `@`,
tab, or CR, which is the OWASP CSV injection list. It is also neutralized when `=`, `+`, `-`, or
`@` follows leading spaces, tabs, CR, or LF, which is Runtime Analyzer's rule. Neutralization
prefixes one apostrophe, then the usual RFC 4180 quoting applies, so `=a,b` becomes `"'=a,b"`. Only
ASCII characters are checked. Full-width lookalikes such as U+FF1D are not, and no product or
vector needed them.

**Numeric cells.** A text cell `-5` is neutralized to `'-5`, because a text path cannot tell a
number from `-2+3`. Callers mark real numbers instead. TypeScript accepts a finite `number`, written
with `String(value)`, or `csvNumeric(text)` for preformatted text such as `1.50`. Rust has
`CsvCell::Numeric` and `From<i64>`, `From<u64>`, and `From<f64>`. A numeric cell must match the
JSON number grammar and is written unchanged; anything else is an error rather than a silent
fallback to text, because a mislabeled cell is a caller bug. Shared vectors use numeric text only,
because JavaScript and Rust format some floats differently (`1e21` against
`1000000000000000000000`). Both forms pass the grammar.

**Opt-out.** `neutralizeFormulas: false` in TypeScript and
`CsvOptions::without_formula_neutralization()` in Rust. The option name states what is turned off;
there is no default-off constructor. Neutralization is lossy: a reader cannot tell a neutralized
`=x` from a user value `'=x`. The opt-out exists for machine-read files that never open in a
spreadsheet, and the READMEs say so.

**Record layout.** Every record ends with CRLF, as RFC 4180 specifies and as all three TypeScript
products already do. A row with one empty cell is written as `""`, as Python's `csv` module does, so
a reader sees one field rather than an empty line. A row with no cells is an empty line. No rows
produce an empty string. Ragged rows are allowed.

**Byte-order mark.** Off by default. `byteOrderMark: true` or `with_byte_order_mark()` writes
U+FEFF once before the first record, even when there are no rows. The vector file stores the mark as
the number `byteOrderMarkCodePoint` (65279) instead of a string escape, because an editor hook in
this repository strips invisible characters and `` escapes from files it touches. Tests build
the mark from that number.

**Unicode.** Strings pass through unchanged, with no normalization, so the composed and decomposed
forms of `é` stay distinct. TypeScript rejects unpaired surrogates, which `TextEncoder` would
otherwise replace with U+FFFD. Rust strings cannot hold them.

## Public types and errors

TypeScript, from `@baukit/data-contracts/export` and the package root: `encodeCsv(rows, options)`,
`csvNumeric`, `CsvCell` (`string | number | CsvNumericCell | null`), `CsvNumericCell`,
`EncodeCsvOptions` (`neutralizeFormulas`, `byteOrderMark`), `CsvEncodeError` with `code`,
`rowIndex`, and `columnIndex`, `CsvEncodeErrorCode` (`invalid_numeric_cell`, `invalid_unicode`,
`unsupported_cell`), `ShareOutcome`, and `SHARE_OUTCOMES`.

Rust, `baukit_core::export`: `encode_csv(rows, options) -> Result<String, CsvEncodeError>`,
`CsvCell` (`Empty`, `Text`, `Numeric`, with `From` for `&str`, `String`, `i64`, `u64`, `f64`, and
`Option<T>`), `CsvOptions` (`new`, `Default`, `without_formula_neutralization`,
`with_byte_order_mark`), and `CsvEncodeError::InvalidNumericCell { row_index, column_index }` with
`code()` returning the shared string.

## Cases

`fixtures/export-csv/csv-encoding-v1.json` holds 27 encoding cases and 12 rejections. It covers
quoting of commas, quotes, CR, LF, and CRLF; empty and null cells; the single-empty-cell row; each
formula prefix, including tab and CR; a formula that also needs quoting; formulas after leading
whitespace; prefix characters later in the cell; numeric cells, including negative, zero, decimal,
and exponent forms; the same text as a neutralized text cell; composed, decomposed, CJK, and
emoji-sequence text; the byte-order mark with and without rows; and the opt-out. Rejections cover
numeric text with a plus sign, a formula, whitespace, a leading zero, missing digits, `NaN`,
`Infinity`, hexadecimal, and a formula with neutralization turned off. Both
`typescript/packages/data-contracts/src/export.test.ts` and
`rust/crates/baukit-core/src/export.rs` run every case. TypeScript adds non-finite numbers,
unpaired surrogates, unsupported runtime values, and iterable input. Rust adds the `From`
conversions and non-finite floats.

## Share outcome comparison

| Source                                                                                     | Outcomes                                                    | Cancellation rule                                                                      | Other errors                                                                                |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Eigenruhe `mobile/src/integrations/sharing.ts:3,78-127`                                    | `shared`, `copied`, `downloaded`, `canceled`, `unavailable` | A rejection named `AbortError` or with "cancel(l)ed" in its message, on native and web | Native: `unavailable`. Web: falls back to download, then clipboard                          |
| Hebkit `mobile/src/integrations/sharing.ts:3,76-132`                                       | Same five                                                   | Same rule                                                                              | Native: `unavailable`. Web: clipboard for text, then download                               |
| Hebkit `mobile/src/features/data-transfer/file-access.ts:23,47-52,127-167`                 | `shared`, `downloaded`, `canceled`, `unavailable`           | Same rule                                                                              | Native: `unavailable`. Web: falls back to download                                          |
| Eigenruhe `mobile/src/features/data-transfer/platform-export.ts`, `platform-export.web.ts` | None; `save` resolves or throws                             | None                                                                                   | Throws "File sharing is unavailable." when sharing is missing; web download always succeeds |
| Tiefgang `mobile/src/integrations/files/export-service.ts:10,85-140`                       | `downloaded`, `shared`, `unavailable`                       | None                                                                                   | Native errors propagate as exceptions                                                       |
| Redemut `mobile/src/account.tsx:590-622`                                                   | Component state `success` or `error`                        | None; `Share.share` result is ignored                                                  | `error`                                                                                     |

The helpers do not agree on cancellation. The two `sharing.ts` forks and Hebkit's `file-access.ts`
share one rule, which comes from the fork. Eigenruhe's own CSV export path, Tiefgang, and Redemut
have no cancelled state at all. Every Expo caller treats a resolved `Sharing.shareAsync` as
`shared` and never inspects a result, so a native sheet the user closed without choosing a target
reaches the product as shared. On the web, `navigator.share` rejects with `AbortError` on
dismissal, so the fork's rule works there. Whether any native path rejects on dismissal is
unverified: no product checkout has `expo-sharing` installed, and the products' native cancellation
tests use mocked rejections. Redemut ignores the `Share.share` result, which reports
`dismissedAction` on iOS, so a dismissed sheet shows success. No product separates "tried and
failed" from "not available". The forks fold both into `unavailable`, and the others throw.

Baukit therefore specifies the outcome type only, with `failed` split from `unavailable` and the
Baukit spelling `cancelled`. It documents that `shared` can include an unreported dismissal. The
products' `copied` outcome is not in the type: it belongs to their text-sharing fallback, not to
file export. `downloaded` maps to `saved`. The Expo implementation stays deferred until two
products agree on how a native dismissal is detected and reported.

## Supported runtimes

The TypeScript encoder targets ES2022 with no browser, Node, or React Native API and no Unicode
property escapes, so browsers, Node 24 or newer, and compatible React Native engines can run it. It returns a string; products write it with their existing file
code. The Rust function uses `std` only and runs wherever `baukit-core` builds, on Rust 1.95.

## Failure behavior

Encoding is all-or-nothing: the first invalid cell throws or returns an error, and no partial output
is returned. Errors carry a stable code and zero-based row and column indices. The encoder never
truncates, retries, or rewrites a rejected cell.

## Privacy boundary

Error messages and fields hold the code and indices, never the cell value, and tests check that a
secret in a rejected cell does not appear in the message. The encoder keeps no state and logs
nothing. Products still own where the file is written, how long it stays in a cache directory, and
whether the share target can read it.

## Breaks

None. The TypeScript root gains new names, which only matters to a consumer that re-exports the
root next to its own `encodeCsv` or `ShareOutcome`.

## Product adoption change

- Eigenruhe: replace `quoteCsvCell` and the row join in `features/data-transfer/csv.ts:67-75` with
  `encodeCsv`, passing planned and actual durations and check-in scores as numbers. Change
  `platform-export.ts` and `platform-export.web.ts` to return `ShareOutcome`, and rename
  `canceled` to `cancelled` and `downloaded` to `saved` in `integrations/sharing.ts`. Map
  non-cancellation errors to `failed`.
- Hebkit: replace `quoteCell` and `writeRows` in `integrations/files/csv.ts:73-82` with
  `encodeCsv`, pass numeric columns as numbers, and teach `parseRows` to strip the apostrophe that
  neutralization adds to text cells, or opt out for this codec after deciding the portable file is
  never opened in a spreadsheet. Keep the `\N` null and `#` metadata quoting local. Rename outcomes
  in `integrations/sharing.ts` and `features/data-transfer/file-access.ts`.
- Tiefgang: replace `quoteCell` and the join in `integrations/files/csv.ts:21-50` with `encodeCsv`,
  passing number columns such as `delta_credits` as numbers so negative values are not
  neutralized. Return `ShareOutcome` from `deliverExportArtifacts` in
  `integrations/files/export-service.ts`.
- Redemut: no CSV to adopt. If it keeps `Share.share`, read `action` and report `cancelled` for
  `Share.dismissedAction` instead of success.
- Runtime Analyzer: replace `csv` in `finops-api/src/routes/audit.rs:82-97` with
  `baukit_core::export::encode_csv`. The output changes from LF to CRLF and from quoting every cell
  to quoting only when needed; update the test at `:175`.
- Solo Leveling System: replace `csv_cell` and the joins in `sl-services/src/orgs.rs:1499-1526`
  with `encode_csv`, passing the aggregates as numeric cells. This fixes the unquoted CR. Update
  the test at `:2231` for CRLF.

The plan's acceptance says two products delete their encoders. That happens in the adoption pass,
not in this Baukit change.

## Follow-up 0.5.1 (2026-09-29)

### Product evidence

Hebkit `797fdff7`, `mobile/src/integrations/files/csv.ts`, could not adopt `encodeCsv`. Its format
writes a null cell as an unquoted `\N` (`NULL` at `:22`) and quotes a literal `\N` text cell so the
reader can tell the two apart (`quoteCell` at `:73-78`). `parseRows` at `:84` reads an unquoted
`\N` as null and a quoted `"\N"` as text. `encodeCsv` wrote null and `''` the same way and had no
way to quote a cell that did not need it, so a Hebkit export read back through its own importer
turned every empty string into null or the reverse.

### Decision

Both options belong in Baukit because the fix is in the quoting rules, which products must not
reimplement next to the encoder.

- `nullMarker` writes null cells as an unquoted marker and quotes any text cell equal to the marker.
  This is the PostgreSQL `COPY ... CSV` convention. With formula neutralization on, a text cell that
  becomes the marker after the apostrophe is added is quoted too. A marker that is empty or contains
  a comma, quote, CR, or LF is rejected with `RangeError` in TypeScript and a panic in Rust, because
  it could not be told apart from data. The Rust builder is a `const fn`, so a bad marker in a
  `const` item fails the build.
- `quoteAllCells` quotes every text and number cell, matching Python's `QUOTE_ALL`. A null cell
  stays unquoted so a marker keeps its meaning.
- A record whose only cell encodes to nothing is still written as `""`, so a row is never a blank
  line that readers skip. With a marker set, a one-cell null row is written as the marker instead,
  so it no longer collides with a one-cell empty string.

Hebkit's quoting of cells that start with `#` stays in the product. It guards Hebkit's own metadata
line, not a CSV rule, and `quoteAllCells` covers it anyway.

Rust `baukit_core::export::CsvOptions` gains `with_all_cells_quoted()` and
`with_null_marker(marker)` with the same rules. Five new cases in
`fixtures/export-csv/csv-encoding-v1.json` pin both encoders to the same bytes.

### Breaks

None. The defaults are unchanged, and the new fields are optional.

### Product adoption change

- Hebkit: replace `quoteCell` and `writeRows` in `mobile/src/integrations/files/csv.ts:73-82` with
  `encodeCsv(rows, { nullMarker: '\\N', quoteAllCells: true, neutralizeFormulas: false })`, passing
  null for null cells. Keep `parseRows`. If the file should stay safe to open in a spreadsheet,
  keep neutralization on and strip the leading apostrophe in `parseRows` instead.
- Tiefgang and Eigenruhe: unchanged from the list above; they need neither option.

### Gates

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D
  warnings`, and `cargo test -p baukit-core --all-features -- --include-ignored` passed in `rust/`.
- `cargo +1.95 check -p baukit-core --all-targets --all-features` passed.
- `corepack pnpm --dir typescript` `build`, `format:check`, `lint`, `test`, and `check` passed.
  The data-contracts suite ran 280 tests.
