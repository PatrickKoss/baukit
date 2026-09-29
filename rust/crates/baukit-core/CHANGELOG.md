# Changelog

All notable changes to `baukit-core` are documented here.

## [Unreleased]

## [0.5.1] - 2026-09-29

### Added

- Add `CsvOptions::with_all_cells_quoted`, which quotes every text and numeric cell and leaves
  empty cells unquoted, and `CsvOptions::with_null_marker`, which writes a marker such as `\N` for
  an empty cell and quotes text with the same content. Either keeps an empty cell apart from empty
  text, so a product's importer can read its own export back. `with_null_marker` panics on an empty
  marker or one holding a double quote, comma, CR, or LF. The shared vectors gain five cases.

## [0.5.0] - 2026-09-28

### Added

- Add the `export` module with `encode_csv`, `CsvCell`, `CsvOptions`, and `CsvEncodeError`. It
  writes RFC 4180 CSV with CRLF separators and prefixes an apostrophe to text cells that start like
  a spreadsheet formula; `CsvOptions::without_formula_neutralization` turns that off and
  `CsvOptions::with_byte_order_mark` adds U+FEFF. `CsvCell::Numeric` holds a JSON-grammar number
  that is written unchanged, so `-5` stays a number. The module uses only `std` and `thiserror`, so
  default dependencies are unchanged. It passes the same vectors as `@baukit/data-contracts/export`.
- Add the optional `pagination` feature and the `pagination` module with `Cursor`, `Page`,
  `PageKey`, `PageParams`, `PaginationError`, `DEFAULT_PAGE_LIMIT`, and `MAX_PAGE_LIMIT`, moved
  from `baukit-http` so domain crates can use them without Axum. The feature adds `base64`,
  `ring`, and `uuid`; the default build keeps its three dependencies.
- Add `pagination::MAX_CURSOR_BYTES` (4096). Breaking: `Cursor::decode` rejects longer input with
  `PaginationError::InvalidCursor` before decoding, and `Cursor::encode` returns the same error
  instead of issuing a longer cursor.
- Add the optional `media-grants` feature and the `media_grant` module with `MediaGrantKey`,
  `MediaGrantKeyRing`, `MediaGrant`, `MediaGrantRequest`, `VerifiedMediaGrant`,
  `MediaGrantKeyError`, `MediaGrantError`, `signing_input`, and `valid_media_path`. It signs and
  verifies HMAC-SHA256 playback grants with a current and an optional previous key, compares
  signatures in constant time, and redacts key material from `Debug`. The feature adds `base64`,
  `ring`, and `zeroize`. It passes `fixtures/media-grants/vectors-v1.json` together with the njs
  verifier in `deploy/media-grants`.

### Changed

- Break: `pagination::Page` serializes `next_cursor` as `nextCursor`.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

### Added

- Add production measurements and checks for trimmed Unicode scalars, compact
  JSON UTF-8 bytes, byte slices, and collection slices.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-core`.
