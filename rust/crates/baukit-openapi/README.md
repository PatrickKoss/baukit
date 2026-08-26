# baukit-openapi

`baukit-openapi` applies Baukit's document conventions to a utoipa-generated OpenAPI schema, supplies
the shared error envelope, and keeps a committed schema file honest with a drift check. Products own
their paths, operations, and endpoint schemas.

```rust
use baukit_openapi::{ErrorEnvelope, OpenApiMetadata, serialize_schema};
use utoipa::openapi::Server;

#[derive(utoipa::OpenApi)]
#[openapi(components(schemas(ErrorEnvelope)))]
struct ApiDoc;

let mut document = <ApiDoc as utoipa::OpenApi>::openapi();
OpenApiMetadata::new("Orders API", "1.2.3", "The Orders service API")
    .servers([Server::new("https://api.example.com")])
    .apply_to(&mut document);

let json = serialize_schema(&document)?;
assert!(json.ends_with('\n'));
# Ok::<(), baukit_openapi::SchemaError>(())
```

Applying metadata preserves product-owned paths, schemas, contact information, license, and existing
security schemes. It fills in the conventions, it does not take the document over.

New metadata uses `/` as its server so one schema describes the service at any deployment origin.
Pass explicit URLs to `servers` when a product needs them. `bearer_auth()` adds the standard bearer
JWT component under `BEARER_AUTH_SCHEME`; an unauthenticated API leaves it off, so the schema never
advertises auth the service does not enforce.

## Drift

A committed schema file is only useful if it matches the code. `serialize_schema` is deterministic:
keys are ordered and the output ends with a newline, so regenerating an unchanged API produces a
byte-identical file and a real change produces a readable diff.

`assert_no_drift` (and its non-panicking `check_no_drift`) compares the generated document against the
committed file. Wire it into a test and CI fails when someone changes a handler and forgets the
schema, instead of a client generator discovering it later:

```rust,no_run
# fn document() -> utoipa::openapi::OpenApi { unimplemented!() }
#[test]
fn openapi_schema_is_current() {
    baukit_openapi::assert_no_drift(&document(), "openapi.json");
}
```

`write_schema` regenerates the committed file. A missing file is treated as empty, so the first
run reports the whole document as drift rather than quietly passing.

## The error envelope

`ErrorEnvelope` and `ErrorBody` are the `{ "error": { "code", "message", "request_id", "details" } }`
shape every Baukit service returns for a failure, and `ResponseEnvelope` is the success side.
`baukit-http` re-exports both and produces them at runtime, so the documented schema and the actual
response body come from one type rather than from a handwritten schema that drifts from the code.

`Rfc3339DateTime` is the timestamp wrapper used in those payloads.

## Scope

No routing, no handlers, no client generation. The crate applies conventions to a document somebody
else generated, and it holds the error contract that HTTP and its consumers share.
