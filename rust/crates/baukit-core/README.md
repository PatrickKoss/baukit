# baukit-core

`baukit-core` holds the handful of types that more than one Baukit crate needs to agree on:
deployment environment, log format, process kind, service identity, build info, resource-budget
measurements, and a CSV export encoder. By default it depends on `serde`, `serde_json`, and `thiserror` and nothing else. The
optional `pagination` feature adds keyset pagination and pulls in `base64`, `ring`, and `uuid`.

## Why this crate exists at all

Configuration and telemetry both need to name the environment a process runs in. Left alone, each
would define its own `Environment` enum, and the moment a value crossed between them somebody would
write a conversion function that silently mapped an unknown string to a default.

Putting the type in one dependency-light crate solves that without the alternative cost. Telemetry
could have owned the vocabulary, but then every configuration-only consumer would inherit the
OpenTelemetry exporter stack to learn what `production` means. `baukit-config` and
`baukit-telemetry` re-export these types from their own APIs, so products import from the crate
they were already using and still get one type.

## Environment and log format

`DeploymentEnvironment` parses and displays as `local`, `testing`, `staging`, or `production`.
An unrecognized name is a `ParseEnvironmentError`, never a fallback to local.

`LogFormat::Auto` resolves against the environment: pretty output locally, newline-delimited JSON
everywhere else. Pick `Json` or `Pretty` explicitly to override.

```rust
use baukit_core::{DeploymentEnvironment, LogFormat};

assert_eq!(
    LogFormat::Auto.resolve(DeploymentEnvironment::Production),
    LogFormat::Json,
);
```

## Service identity

`ServiceIdentity` pairs a product name with a `ProcessKind` and produces the canonical service
name as `<product>-<process>`:

```rust
use baukit_core::{DeploymentEnvironment, ProcessKind, ServiceIdentity};

let identity = ServiceIdentity::new(
    "orders",
    ProcessKind::Api,
    "1.2.3",
    "abc123",
    DeploymentEnvironment::Production,
);

assert_eq!(identity.service_name(), "orders-api");
```

The API and worker processes of one product are separate services in logs, metrics, and traces, and
deriving the name from a shared type is what keeps them from disagreeing. `ProcessKind` covers
`Api`, `Worker`, `Migrate`, and `Seed`. `BuildInfo` carries version, commit, and the Rust version
used to compile the process; `baukit-runtime`'s `build_info!` macro fills it from the binary crate's
own Cargo metadata.

## Resource-budget measurements

The `limits` module measures trimmed Unicode scalar values, compact JSON UTF-8 bytes, byte slices,
and collection slices. A check returns the measured and allowed values. Products map
`LimitExceeded` into their own error code and keep the limit itself in product configuration.

```rust
use baukit_core::limits::{check_compact_json_utf8_bytes, check_trimmed_unicode_scalars};
use serde_json::json;

let text = check_trimmed_unicode_scalars("  e\u{301}  ", 2)?;
assert_eq!(text.measured(), 2);

let document = check_compact_json_utf8_bytes(&json!({"value": "é"}), 14)?;
assert_eq!(document.allowed(), 14);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Trimming uses Rust's Unicode whitespace definition. It does not normalize text, so `é` is one
scalar and `e` followed by a combining acute accent is two. Compact JSON uses `serde_json` without
pretty printing.

### Migration from `baukit-test`

Production code should replace `baukit_test::trimmed_text_length` with
`baukit_core::limits::trimmed_unicode_scalar_count`, and replace
`baukit_test::compact_document_bytes` with `baukit_core::limits::compact_json_utf8_bytes`. The old
`baukit-test` names remain available and delegate to these functions.

## CSV export

`export::encode_csv` writes RFC 4180 CSV for user-facing exports. Every record ends with CRLF, and a
cell is quoted when it contains a comma, a double quote, CR, or LF. A row holding one empty cell is
written as `""` so a reader still sees one field.

```rust
use baukit_core::export::{CsvCell, CsvOptions, encode_csv};

let rows = [
    vec![CsvCell::from("note"), CsvCell::from("amount")],
    vec![CsvCell::from("=HYPERLINK(\"https://example.test\")"), CsvCell::from(-5_i64)],
];
let csv = encode_csv(&rows, CsvOptions::new())?;
assert_eq!(csv, "note,amount\r\n\"'=HYPERLINK(\"\"https://example.test\"\")\",-5\r\n");
# Ok::<(), baukit_core::export::CsvEncodeError>(())
```

Formula neutralization is on by default. A text cell gets a leading apostrophe when its first
character is `=`, `+`, `-`, `@`, tab, or CR, or when `=`, `+`, `-`, or `@` follows leading spaces,
tabs, CR, or LF. Only these ASCII characters are checked. The apostrophe changes the value, so a
file that your own importer reads back must either strip it or be written with
`CsvOptions::without_formula_neutralization()`. Use the opt-out only for machine-read files that
never open in a spreadsheet.

Text that looks like a negative number, such as `"-5"`, is neutralized. Mark real numbers as
`CsvCell::Numeric`, or convert from `i64`, `u64`, or `f64`. A numeric cell must match the JSON
number grammar and is written unchanged; anything else, including `NaN` and infinities, fails with
`CsvEncodeError::InvalidNumericCell` carrying the row and column index but not the value.
`CsvOptions::with_byte_order_mark()` starts the output with U+FEFF for spreadsheet programs that
need it to detect UTF-8.

The shared vectors live in `fixtures/export-csv/csv-encoding-v1.json`; `@baukit/data-contracts`
passes the same file.

## Keyset pagination

Enable the `pagination` feature to use `baukit_core::pagination` from domain and service crates
without depending on Axum:

```toml
baukit-core = { version = "0.4", features = ["pagination"] }
```

`PageParams` validates `limit` and carries the still-encoded cursor. `Page::from_rows` truncates an
over-fetched row set and issues the next `Cursor`. The cursor is base64url JSON holding a version,
the keyset position, and a short hash of the normalized request filters, so a cursor replayed
against other filters fails with `PaginationError::InvalidCursor`.

`Cursor::decode` rejects input longer than `MAX_CURSOR_BYTES` (4096 bytes) before it base64-decodes
or parses anything, so an oversized query parameter costs no allocation. `Cursor::encode` returns
the same error rather than issue a cursor that `decode` would reject. `baukit-http` converts
`PaginationError` into a field-level `validation_failed` error; its README has a handler example.

## Scope

No exporters, no async runtime, no HTTP framework, no operational routing, and no product limit
policy. A type or function earns a place here only when two crates would otherwise define it twice.
Everything else belongs in the crate that owns the behavior.
