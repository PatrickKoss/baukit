# Changelog

All notable changes to `baukit-core` are documented here.

## [Unreleased]

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Update the UUID dependency baseline to 1.27. Keep Rust 1.95 as the MSRV.

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

## [0.6.0] - 2026-10-02

## [0.5.2] - 2026-09-30

### Added

- Add the optional `webhook-signature` feature and the `webhook_signature` module with
  `webhook_signing_input`, `sign_webhook_hmac_sha256`, `verify_webhook_hmac_sha256`,
  `WEBHOOK_SIGNATURE_VERSION`, and `WEBHOOK_SIGNATURE_PREFIX`. Senders and receivers now apply the
  `baukit-webhook-v1` signature at runtime instead of copying it from `baukit-test`. The feature
  adds `base64` and `ring`. `fixtures/webhooks/signature-v1.json` runs against it.
- Add a tie-breaker type parameter to `pagination::PageKey<T, K = Uuid>` and
  `Cursor::page_key_as::<T, K>()`. `Cursor::from_page_key` and `Page::from_rows` accept any
  `K: Display`, so a list ordered by a composite key such as `(source, target)` or by a non-UUID
  ID keeps the filter binding, version check, and size bound. `PageKey<T>` and
  `Cursor::page_key::<T>()` still mean a UUID tie-breaker, and the cursor wire format is
  unchanged.

### Changed

- `fixtures/media-grants/vectors-v1.json` gains an `absolute-form-target` case: a path carrying a
  scheme and host is `invalid_path`. The nginx verifier in `deploy/media-grants` now refuses a
  request whose request line has an absolute-form target (`GET http://host/path`).

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
