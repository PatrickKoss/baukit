# baukit-core

`baukit-core` holds the handful of types that more than one Baukit crate needs to agree on:
deployment environment, log format, process kind, service identity, build info, and resource-budget
measurements. It depends on `serde`, `serde_json`, and `thiserror` and nothing else.

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

## Scope

No exporters, no async runtime, no HTTP framework, no operational routing, and no product limit
policy. A type or function earns a place here only when two crates would otherwise define it twice.
Everything else belongs in the crate that owns the behavior.
