# Migrate MCP tools to the Rust remote server

Baukit now generates only Rust MCP servers. `--mcp` requires `--backend
--auth oidc` and adds `{name}-mcp` to the backend workspace. The backend
serves Streamable HTTP at `/mcp`. There is no legacy transport flag.

This guide is based on the current `mcp/src` code in Leitbild, Hebkit,
Redemut, Eigenruhe, Tiefgang and Schlauzug, and SLS's
`solo-leveling-system-mcp` crate and `compose/mcp.rs`. Product repositories
were read without changing them. No product is live, so remove the old
server when adopting the Rust implementation.

## Add the backend crate

Generate a reference product with the same name in a separate directory:

```sh
baukit new NAME --backend --web --mobile --mcp --auth oidc --dir .mcp-reference
```

Compare the output with the product. Copy the MCP crate, `backend/mcp-tools.json`,
`backend/tests/tool_drift.rs`, and `docs/remote-mcp.md`. Merge the workspace
member and dependencies, binary composition, config, Keycloak inputs,
Helm values and ingress routes. Keep the product's existing services and
repositories. The reference `list_items` tool is an example to replace.

Set `mcp = true` under `[capabilities]` in `baukit.toml`. Remove the previous
inline table or `[capabilities.mcp]` table, including `authentication` and
`transport`. Keep `auth = "oidc"` and `backend = true`. Remove
`mcp/src/api/schema.d.ts` from `openapi.consumers`; retain declarations used
by web and mobile. Doctor reports old capability tables and remaining
TypeScript servers with this guide's path before typed manifest parsing.

## Port each tool

Keep the public tool name and argument spelling unless a client-facing
break is deliberate. Inventory the old read and write registries and the
actual registration calls. Redemut keeps schemas in `server.ts`; Schlauzug
has a single `tools.ts` instead of split registries.

1. Put the tool's description, input JSON Schema, output JSON Schema,
   required scopes and `read_only` flag in `ScopedTool`. Keep `definitions()`
   as the committed schema source and return it from `ToolService::tools`.
   Translate Zod objects, enums, tagged unions, defaults, optional fields,
   bounds and formats. Match strict objects with `additionalProperties: false`.
   Preserve `structuredContent` fields through the returned JSON object.
2. Deserialize arguments into product input types in `ToolService::call`.
   Use `#[serde(deny_unknown_fields)]` for strict objects. JSON Schema
   advertises the contract; it does not validate every business constraint.
   Reimplement Zod refinements such as real calendar dates, zoned timestamps,
   Unicode character limits and conditional confirmation in domain or
   service validation. Add rejection tests for each refinement.
3. Define a service port with domain inputs and outputs. The generated
   `ItemReadService` shows the pattern. Implement it using the product service
   already called by REST. Use `Principal::subject()` and `Principal::issuer()`
   to resolve the product account; never accept an owner or subject supplied
   in tool arguments. Keep Axum, rmcp, JWTs and HTTP headers outside tool logic.
4. Replace REST route calls with service calls. Move useful projections,
   pagination, redaction and safe error codes from the TS client into the
   adapter. Do not expose raw service, database or provider error text.
   Replace `AsyncLocalStorage` request state with explicit service inputs.
   Preserve bounded external calls and the service's cancellation policy.
5. Add service-port tests for identity, output shape and invalid arguments.
   Add HTTP protocol tests for listing and calling the tool, including a
   missing scope and a second account. Generate and review the schema:

```sh
cargo run --manifest-path backend/Cargo.toml -p NAME-mcp --bin mcp-tools > backend/mcp-tools.json
cargo test --manifest-path backend/Cargo.toml -p NAME-mcp --test tool_drift
```

The reusable server rejects missing scopes with HTTP 403 before dispatch
and filters `tools/list` by the principal's grants. Each required scope
must also be advertised in `mcp.scopes_supported` and issued by Keycloak.
Empty required-scopes lists are rejected at registration. Use explicit
grants for reads and writes.

For writes, set `read_only = false` and require a separate write scope.
Baukit gives writes a conservative destructive annotation and does not
claim idempotence. Keep confirmation, idempotency and ownership checks in
product services. Preserve caller-supplied intent keys, operation IDs,
revision preconditions and replay results. A transport change must not
turn an uncertain write into an automatic new attempt. Add tests for
repeated identical arguments, changed arguments with the same key, stale
revisions, and calls against another account's records.

The server defaults to a 32 KiB request limit. Review larger draft tools
against that limit and configure a bounded `max_request_body_bytes` if
needed. Keep product output limits and pagination. Schlauzug's complete
draft and SLS's 256 KiB result bound need explicit review.

## Port errors, resources and prompts

| TypeScript pattern | Rust API |
| --- | --- |
| `return {isError: true, content, structuredContent}` | Return `Err(ToolError::new(code, text).with_structured_content(payload))`. The message supplies text and the payload supplies `structuredContent`. |
| `return {isError: true, content}` | Return `Err(ToolError::new(code, text).text_only())`. Keep the existing output schema. |
| `registerResource(name, uri, metadata, callback)` | Add `ScopedResource { resource: Resource::new(uri, name), required_scopes }` to `ResourceService::list`. Copy title, description and MIME type with the resource builders. Implement `read(principal, uri)`. |
| `registerResource(name, new ResourceTemplate(uriTemplate, {list: undefined}), metadata, callback)` | Add `ScopedResourceTemplate` to `ResourceService::templates`. Leave the concrete list empty when the old callback did not list resources. Implement `read` using the matched URI. |
| `registerPrompt(name, {description, argsSchema}, callback)` | Add `ScopedPrompt` to `PromptService::list`, with `PromptArgument` metadata. Implement `get(principal, name, arguments)` and return `PromptResult` messages. Use `None` for an argument-free prompt. |
| `throw new McpError(InvalidParams, message, data)` in a resource or prompt | Return `CapabilityError::InvalidParams { message, data }`. Use `CapabilityError::Internal` for a safe service failure. |

Eigenruhe's strict tool envelope remains `{data,error}`. Successes return
`json!({"data": value, "error": null})`. Errors return the same envelope with
`data: null`, a safe error object and `with_structured_content`. Keep its
65,536-byte output bound, clipped strings, status, request ID and allowed
stale-revision details in the product adapter. Test both envelopes against
its output schema. Preserve `eigenruhe://content/{id}` and
`eigenruhe://programs/{id}` as templates named `content` and `program`,
including titles, descriptions and `application/json`. Use `content:read`
and the product's program read grant. Keep UUID validation and account checks
in the read service; return the original URI with JSON text contents.

Hebkit's `exercise` and `training-plan` templates remain
`hebkit://exercises/{id}` and `hebkit://plans/{id}`. Copy their titles,
descriptions and MIME types into `ResourceTemplate` metadata and attach the
catalog and training read grants. Preserve its UUID validation and safe
not-found error details in `read`. Return JSON text using the requested URI.
Both products currently omit a resource listing callback, so their concrete
`ResourceService::list` registries stay empty.

Redemut's `recommend-next-steps` prompt has no arguments. Register a
`ScopedPrompt` with `Prompt::new("recommend-next-steps", Some(description), None)`
and the learning/content read grants. Return its existing user-role text
through `PromptResult::new(vec![PromptMessage::new_text(Role::User, text)])`.
Keep the instructions to read stats, the current plan and available content,
and to ask before setting a plan.

Schlauzug currently returns text-only tool errors. Serialize its safe
`code`, `message`, optional `requestId` and `retryAfterSeconds` object into
`ToolError::new(code, serialized_error).text_only()`. This retains the
`isError` flag and text block without adding an envelope. Keep its existing
output schemas. The [MCP error examples](https://modelcontextprotocol.io/specification/2026-07-28/server/tools#error-handling)
use text-only errors, and the [TypeScript SDK skips their output-schema validation](https://github.com/modelcontextprotocol/typescript-sdk/blob/main/docs/servers/errors.md).
No resources or prompts are needed.

```rust
let services = baukit_mcp::McpServices::new(tools)
    .with_resources(resources)
    .with_prompts(prompts);
let mcp = baukit_mcp::router(config, services, store, policy).await?;
```

The generated MCP crate's `services` function supplies the router's second
argument. Add optional services there. Attach only services the product uses.
Resource templates support simple
`{name}` path segments, which cover the Eigenruhe and Hebkit URIs above.
All lists filter by the policy's effective scopes; reads and gets require
all declared grants. These checks do not replace record ownership checks.

Extend the product's committed definition export with `capability_schema`
or `service_schema`, and keep `tool_drift` as the combined contract test.
`resource_schema` includes concrete resource metadata, template URIs and grants;
`prompt_schema` includes prompt descriptions, arguments and grants. Add HTTP
rejection tests and a real Keycloak resource read. See
[resources and prompts](../remote-mcp.md#add-resources-and-prompts) for the port
signatures and result constructors.

## Change Keycloak and deployment

Copy the reference realm's public `NAME-mcp` client and reconciliation
entry. Enable the authorization code flow with PKCE `S256`, disable direct
access grants and service accounts, and register exact redirects. The
reference includes Claude Code's loopback callback on port 18888 and
`https://claude.ai/api/mcp/auth_callback`. Add another callback only after
checking the client's actual URL. Retire device-flow MCP clients if no
other product client uses them. Keep web/mobile clients such as `hebkit-app`.

The template uses a pre-registered public client, with no client secret.
It does not enable dynamic client registration. Explicit registration
works with Keycloak and avoids granting clients permission to create
arbitrary registrations in the product realm.

Replace the example `items:read` scope with the product's read scopes and
add separate write scopes. Include the scopes in the client's allowed
client scopes and token `scope` claim. Keep standard `basic`, `openid`,
`profile` and email mappings needed for `sub` and existing application
clients. A realm import does not replace an existing realm, so apply the
reviewed changes through the product's reconciliation or realm tooling.

Set the MCP audience mapper to the canonical public resource URL, for
example `https://api.example.com/mcp`. Use that exact URL for
`mcp.resource_url`, Helm `mcp.resourceUrl` and the OAuth resource request.
An application audience such as `tiefgang-app` is not the MCP resource.
Tokens must have the issuer, resource audience, subject and unexpired
expiry required by `baukit-auth`. API keys and static personal tokens
cannot replace resource OAuth access tokens.

Set `mcp.enabled`, issuer, exact allowed hosts and exact allowed origins.
Environment lists are JSON arrays under `NAME__MCP__ALLOWED_HOSTS` and
`NAME__MCP__ALLOWED_ORIGINS`. Expose `/mcp`,
`/.well-known/oauth-protected-resource` and its `/mcp` suffix route through
the existing backend ingress. Use HTTPS and shared Redis rate limiting
outside local development. Do not derive the resource or issuer from
untrusted request headers.

Clients discover the protected resource metadata after a 401 challenge,
obtain an authorization code with PKCE, exchange it for a token for the
resource and requested scopes, then send the bearer token to `/mcp`.
Follow [remote MCP](../remote-mcp.md) for Claude Code and Claude Desktop
connection instructions. Remove old command-based MCP client entries.

## Product checklist

| Product | Current implementation | Work to preserve while porting |
| --- | --- | --- |
| Leitbild | 10 reads and 7 writes. Device flow through `leitbild-mcp`, cached login and `LEITBILD_API_TOKEN` override. | Port journal, program/run, reflection and job projections from `api/validation.ts`. Preserve cursor and locale handling, real dates, Markdown character limits, `intentKey`, `expectedRevision`, stage completion and asynchronous reflection job receipts. Split journal, program and reflection read/write grants. Remove the device-login commands and cache after the client uses remote OAuth. |
| Hebkit | 38 reads and 42 writes. Device flow through the shared `hebkit-app` client, cached login and `HEBKIT_API_TOKEN` override. | Add a separate MCP public client without replacing `hebkit-app`. Port training, workout, nutrition, catalog, hydration and shopping service ports. Preserve diary-local dates and time zones, `rev-N` revision preconditions, operation/copy batch IDs and explicit `confirmReplace` for destructive nutrition copies. Keep seven-day copy bounds, planned/consumed states and shopping units. Replace the 60-second ambient operation signal with bounded service calls. |
| Redemut | 8 reads and 8 writes. Personal-token and caller-module providers remain in source; the CLI uses device flow. It also serves a separate TS HTTP endpoint with JWT verification and scope checks. | Replace both TS transports and the HTTP auth/config code with the backend mount. Retain `learning:read`, `content:read`, `content:write`, `plan:write` and `account:read` mappings. Port learning plans, dialogs, word packs, quotas, nested node/choice/word schemas, output schemas, idempotency keys and expected revisions. Replace the old `localhost:3001/mcp` audience with the backend resource URL. Remove the separate server port and session/concurrency implementation. |
| Eigenruhe | 23 reads and 17 writes. Device flow through `eigenruhe-cli`, issuer/profile-dependent token cache and `EIGENRUHE_API_TOKEN` override. | Add a dedicated MCP public client; retain the CLI client if another Node client uses it. Port program previews/adaptation, practice/check-ins, settings, content, connections and soundscape tools. Preserve output validation, idempotency keys, If-Match, adaptation diff preconditions, asynchronous operation receipts, time units and provider side effects. Keep account fences and service quotas. |
| Tiefgang | 11 reads and 7 writes. Device flow through `tiefgang-mcp`, cached login, `TIEFGANG_API_TOKEN` override and `tiefgang-app` audience. | Change the audience to the MCP resource URL. Port focus sessions, guard profiles/rules, projects, XP/credit and weekly summaries. Preserve ISO timestamps, the 15-second service deadline, revision-based session transitions and idempotency keys. Aborting a session remains destructive; a write grant does not bypass strict-mode domain rules. |
| Schlauzug | 6 reads and 3 default writes, plus opt-in `draft_delete`. Static `SCHLAUZUG_ACCESS_TOKEN` and a newer TS server SDK. | Add a Keycloak MCP client and read/draft-write scopes. Port catalog, owned drafts, progress and history. Preserve opaque account-bound page tokens, field redaction, five-question slices, no answer-key disclosure, strict question-format tagged unions and root `limits.json` bounds. Keep draft versions, UUID operation IDs, 24-hour replay semantics and the deletion opt-in. No publishing, purchases, account deletion or gameplay tools are introduced. |
| SLS | Its own Rust rmcp server, 4 reads and 2 writes. Per-request JWT validation, introspection, issuer/subject account lookup, erasure fence and product quota checks. | Replace generic transport, Host/Origin protection, metadata, challenges, body limits and scope dispatch with `baukit-mcp`. Keep `MeService`, quest catalog and user-quest ports, account lookup, deleted-user checks, erasure fences, quota receipts, private projections and bounded output. Preserve `profile:read`, `quests:read`, `quests:write`, absolute task-progress writes and stable idempotency keys. |

SLS can use `AuthenticationPolicy` before both `tools/list` and `tools/call`.
Compose `KeycloakIntrospectionPolicy` with SLS's account, user, erasure and
quota ports. Set `cache_ttl = Duration::ZERO` to preserve its every-request
introspection. Run the erasure fence against the verified issuer/subject,
resolve the linked active user, and retain the personal quota check.
Return the introspected principal with `with_subject(user.id.to_string())`;
its effective scopes drive discovery and dispatch. Keep database and erasure
checks outside the introspection cache. Map erased or deleted users to
`PolicyDenial::Inactive`, storage failures to `Unavailable`, and exhausted
quotas to `RateLimited`. No SLS domain type enters Baukit.

Use `profile:read`, `quests:read` and `quests:write` in the tool definitions.
Keep SLS's service authorization, pagination, projections, bounded output,
write preconditions and replay keys. Baukit replaces its JWT/introspection
HTTP adapter, host/origin checks, metadata, challenges, body limits and scope
dispatch. See [authentication policy](../remote-mcp.md#authentication-policy)
for credentials, cache settings and the generated composition hook.

## Remove the old server and verify

Delete the product's `mcp/` TypeScript package after porting its tools and
tests. Remove its lockfiles, package scripts, bin entries, token-cache and
provider-module settings, generated API copies, route allowlist/doc scripts,
stdio tests, CI job and command-based client configuration. Remove
`--mcp-transport` and `--mcp-auth` from generation scripts. Retain TS packages
used by web/mobile; `@baukit/auth-node` is still used by web OIDC tests.

SLS instead deletes its duplicated rmcp server and HTTP middleware after
its composed policy passes the revocation, account and erasure tests. Its product services and account policy
remain in SLS. Product repositories are not migrated by this Baukit change.

```sh
baukit doctor
cargo fmt --manifest-path backend/Cargo.toml --all --check
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path backend/Cargo.toml -- --include-ignored
cargo test --manifest-path backend/Cargo.toml -p NAME-bin --test openapi_drift
cargo test --manifest-path backend/Cargo.toml -p NAME-mcp --test tool_drift
```

Run a real client against Keycloak and the backend. Verify initialize,
`tools/list`, a read call and an authorized write, then wrong audience,
expired token and missing write scope. Confirm a stale revision and a
replayed intent cannot apply another effect. Commit the reviewed schema
and remove the reference directory.
