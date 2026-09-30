# baukit-core

`baukit-core` holds the handful of types that more than one Baukit crate needs to agree on:
deployment environment, log format, process kind, service identity, build info, resource-budget
measurements, and a CSV export encoder. By default it depends on `serde`, `serde_json`, and `thiserror` and nothing else. The
optional `pagination` feature adds keyset pagination and pulls in `base64`, `ring`, and `uuid`. The
optional `media-grants` feature adds signed media grants and pulls in `base64`, `ring`, and
`zeroize`. The optional `webhook-signature` feature adds the `baukit-webhook-v1` signature and
pulls in `base64` and `ring`.

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

By default `CsvCell::Empty` and empty text both write an empty field. For a file your own importer
reads back, `CsvOptions::with_all_cells_quoted()` quotes every text and numeric cell and leaves empty
cells unquoted, and `CsvOptions::with_null_marker("\\N")` writes the marker for an empty cell and
quotes text with the same content. The marker is a `&'static str`; an empty marker or one holding a
double quote, comma, CR, or LF panics, and fails the build when the options are a `const` item.

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
against other filters fails with `PaginationError::InvalidCursor`. `Page` serializes as
`{ "items": [...], "nextCursor": "..." }`, with `nextCursor` set to `null` on the last page.

`PageKey<T, K = Uuid>` holds the sort value and the tie-breaker. Most lists break ties on the row
UUID and read the position back with `cursor.page_key::<T>()`. A list ordered by a composite or
non-UUID key picks another `K`, for example `PageKey::new(source, target)` for rows ordered by two
text columns, and reads it back with `cursor.page_key_as::<String, String>()`. The tie-breaker is
stored through `Display` and parsed back through `FromStr`. A list ordered only by its ID passes
the ID as both parts. Bind the tenant or other scope by putting it in the normalized filters.

`Cursor::decode` rejects input longer than `MAX_CURSOR_BYTES` (4096 bytes) before it base64-decodes
or parses anything, so an oversized query parameter costs no allocation. `Cursor::encode` returns
the same error rather than issue a cursor that `decode` would reject. `baukit-http` converts
`PaginationError` into a field-level `validation_failed` error; its README has a handler example.

## Signed media grants

Enable the `media-grants` feature to sign short-lived URLs for media that an edge proxy serves
from object storage:

```toml
baukit-core = { version = "0.4", features = ["media-grants"] }
```

A grant is the query `expires=<unix seconds>&keyId=<id>&mode=playback&signature=<base64url>`. The
signature is unpadded base64url of HMAC-SHA256 over `"{path}\n{expires}\nplayback\n{keyId}"`, keyed
with the base64url-decoded secret. `media_grant::signing_input` returns those exact bytes.

```rust
use baukit_core::media_grant::{MediaGrantKey, MediaGrantKeyRing, MediaGrantRequest};

let current = MediaGrantKey::from_base64url(
    "current_2026_09",
    "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY",
)?;
let ring = MediaGrantKeyRing::new(current, None)?;
let now = 2_000_000_000;
let grant = ring.sign("/media/clip.mp4", now + 300, now)?;
assert_eq!(
    grant.query(),
    "expires=2000000300&keyId=current_2026_09&mode=playback\
     &signature=mBO757nN11v6RkL2jFteEiNklevsoA7G1Rl-yHoL9gg",
);

let query = grant.query();
let verified = ring.verify(MediaGrantRequest {
    method: "GET",
    path: "/media/clip.mp4",
    query: &query,
    now,
})?;
assert_eq!(verified.key_id(), "current_2026_09");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`MediaGrantKey::from_base64url` takes a canonical unpadded base64url secret that decodes to at
least 32 bytes and a key ID matching `[A-Za-z0-9][A-Za-z0-9_-]*` of at most 64 bytes. Generate a
secret with `openssl rand 32 | basenc --base64url | tr -d '='`. `MediaGrantKeyRing` signs with the
current key and verifies grants from either.

Build one `MediaGrantKeyRing` at startup and share it; there is no reload API. To rotate, restart
with the new key as current and the old key as previous. Roll the edge verifier first, so it knows
the new key before the first grant signed with it arrives. Once the longest grant lifetime has
passed, restart again without the previous key.

The signer issues grants at most `MAX_GRANT_LIFETIME_SECONDS` (3600) ahead. The verifier accepts an
expiry up to 60 seconds further, so a verifier clock that lags the signer's still accepts a fresh
grant. There is no grace after expiry: `expires <= now` fails with `expired`.

Paths must already be normalized: a leading `/`, at most 512 bytes, and segments of
`[A-Za-z0-9._-]` that do not start with `.`. Percent-encoding, empty segments, dot segments, and
backslashes fail with `invalid_path`, so a verifier never decodes or normalizes before it checks
the signature. Keep the product's allowlist of media paths in the proxy's routing. The query must
hold exactly the four parameters in the order above.

The verifier compares signatures with `ring::hmac::verify` in constant time. `Debug` output for
keys, grants, and requests redacts secrets, signatures, and queries, and error messages name only
the failed check. `MediaGrantError::code` and `MediaGrantKeyError::code` return the snake_case
codes the edge verifier uses. `deploy/media-grants` has the njs verifier for nginx. Both pass
`fixtures/media-grants/vectors-v1.json`, whose expected signatures come from an independent
generator.

## Webhook signature

Enable the `webhook-signature` feature to sign outbound webhooks and verify them on the receiving
side:

```toml
baukit-core = { version = "0.5", features = ["webhook-signature"] }
```

`webhook_signature::sign_webhook_hmac_sha256` signs the delivery with HMAC-SHA256 over the version
line `baukit-webhook-v1`, the Unix timestamp, the byte length of the delivery ID, the delivery ID,
and the raw body, each field before the body ending in `\n`. It returns `v1=` followed by the
unpadded base64url tag. `webhook_signing_input` returns the exact signed bytes.

```rust
use baukit_core::webhook_signature::{sign_webhook_hmac_sha256, verify_webhook_hmac_sha256};

let body = br#"{"event":"created"}"#;
let signature = sign_webhook_hmac_sha256(b"current-secret", 1_800_000_000, "delivery-7", body);
assert_eq!(signature, "v1=UpNJdPkf1wS7p7DY75L8nz7Rz_BUPFFlEOX3ma4py7w");
assert!(verify_webhook_hmac_sha256(
    [b"previous-secret".as_slice(), b"current-secret".as_slice()],
    1_800_000_000,
    "delivery-7",
    body,
    &signature,
));
```

Header names stay product configuration: send the signature, the timestamp, and the delivery ID in
separate headers. Every retry of one delivery reuses the timestamp, delivery ID, and body, so its
signature does not change. `verify_webhook_hmac_sha256` takes the current key and any key still in
its rotation overlap, compares in constant time, and returns `false` for a missing prefix, padding,
hex, or a truncated tag. The timestamp window and delivery ID dedupe are receiver policy and are
not checked here. `fixtures/webhooks/signature-v1.json` pins the signing bytes, signatures, and
verification results for other runtimes.

## Scope

No exporters, no async runtime, no HTTP framework, no operational routing, and no product limit
policy. A type or function earns a place here only when two crates would otherwise define it twice.
Everything else belongs in the crate that owns the behavior.
