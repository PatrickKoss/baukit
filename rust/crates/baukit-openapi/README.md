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

## camelCase names

Every property and every path and query parameter in a Baukit API is lower camelCase: a lowercase
ASCII letter followed by ASCII letters and digits. `check_camel_case_names` serializes the document
and returns `SchemaError` with one `NamingViolation` per offending name, each carrying its JSON
pointer. `assert_camel_case_names` panics with the same list:

```rust,no_run
# fn document() -> utoipa::openapi::OpenApi { unimplemented!() }
const STANDARD_NAMES: &[&str] = &["access_token", "token_type", "expires_in"];

#[test]
fn openapi_names_are_camel_case() {
    baukit_openapi::assert_camel_case_names(&document(), STANDARD_NAMES);
}
```

The check skips enum and const values, defaults, examples, discriminator mappings, `x-`
extensions, and header and cookie parameters. `additionalProperties` map keys never appear in a
document, so they cannot fail it. The exemption list is for names a standard defines, such as the
OAuth 2.0 token response fields, and applies wherever the name occurs. `find_naming_violations`
runs the same walk on any `serde_json::Value`, which is how you check a committed `openapi.json`
without generating it.

## The error envelope

`ErrorEnvelope` and `ErrorBody` are the `{ "error": { "code", "message", "requestId", "details" } }`
shape every Baukit service returns for a failure, and `ResponseEnvelope` is the success side.
`baukit-http` re-exports both and produces them at runtime, so the documented schema and the actual
response body come from one type rather than from a handwritten schema that drifts from the code.

`Rfc3339DateTime` is the timestamp wrapper used in those payloads.

## Revision preconditions

`document_if_match` and `document_etag` document the `baukit-http` revision precondition on an
operation the product already generated, so the `If-Match` parameter and the `ETag` header are not
written by hand in every route:

```rust
use baukit_openapi::{IfMatchRequirement, document_etag, document_if_match};
use utoipa::openapi::path::Operation;

let mut operation = Operation::new();
document_if_match(&mut operation, IfMatchRequirement::Required);
document_etag(&mut operation);
assert!(operation.responses.responses.contains_key("428"));
```

`document_if_match` replaces any `If-Match` header parameter, whatever its case, with a string
parameter whose description names the three error codes. It adds 400 `invalid_if_match` and 412
`precondition_failed` responses, plus 428 `precondition_required` when the header is required.
Each uses the shared error envelope. A response the operation already documents stays as it is.
`document_etag` adds the `ETag` header to every inline 2xx response and skips `$ref` responses.
`if_match_parameter` and `etag_header` return the two pieces for products that assemble operations
themselves. The error codes are exported as `PRECONDITION_REQUIRED_CODE`,
`PRECONDITION_FAILED_CODE`, and `INVALID_IF_MATCH_CODE`, and `baukit-http` uses the same constants.

## Error responses

`ErrorResponseRules` documents the error responses and response headers a product's middleware adds
to every route, so handlers only document the statuses they return themselves. Each rule pairs a
condition with a status or a header. The product picks the statuses and writes every description;
the crate only decides which operations match:

```rust
use baukit_openapi::{ErrorResponseRules, OperationCondition as When};
use utoipa::openapi::path::HttpMethod;

#[derive(utoipa::OpenApi)]
struct ApiDoc;

let mut document = <ApiDoc as utoipa::OpenApi>::openapi();
ErrorResponseRules::new()
    .status(When::HasRequestBody, 422, "The request body does not match the schema.")
    .status(When::Secured, 401, "A valid bearer credential is required.")
    .status(When::HasPathParameter, 404, "The resource was not found.")
    .status(When::Always, 500, "An internal error occurred.")
    .standard_headers()
    .apply(&mut document);

ErrorResponseRules::new()
    .status(When::Always, 412, "The resource changed since it was read.")
    .apply_where(&mut document, |path, method| {
        *method == HttpMethod::Patch && path.starts_with("/v1/widgets/")
    });
```

| Condition | An operation matches when |
|---|---|
| `Always` | always |
| `Secured` | its `security`, or the document's when it has none, names a scheme. `[{}, {bearer}]` counts; `[{}]` alone does not |
| `HasRequestBody` | it declares a request body |
| `HasPathParameter` | its path template contains `{` |
| `UnsafeMethod` | the method is `POST`, `PUT`, `PATCH`, or `DELETE` |
| `HasIdempotencyKey` | it declares an `Idempotency-Key` header parameter, in any case |

A status rule adds a response with the shared error envelope only when the operation does not
document that status yet, so a handler's own 404 description wins. A header rule adds the header to
the selected inline responses (`ResponseSelector::Every` or `ResponseSelector::Status(429)`) unless a
header of that name, compared without case, is already there. Responses given as a `$ref` are left
alone. Applying the same rules twice changes nothing.

`standard_headers()` adds what `baukit-http` and `baukit-auth` send: `X-Request-Id` on every
response, `Retry-After` (integer seconds, minimum 0) on 429, and `WWW-Authenticate` on 401.
`request_id_header`, `retry_after_header`, and `www_authenticate_header` return the same headers
for products that add them to a custom selector.

Call `document_if_match` before applying the rules. The precondition 400 then keeps its
`invalid_if_match` description, and the header rules still add `X-Request-Id` to 400, 412, and
428.

## Scope

No routing, no handlers, no client generation. The crate applies conventions to a document somebody
else generated, and it holds the error contract that HTTP and its consumers share.
