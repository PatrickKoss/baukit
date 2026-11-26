# Remote MCP

This product's MCP crate lives in the backend. The generated template starts
with MCP disabled, and the backend mounts `/mcp` when `mcp.enabled` is true.
Baukit's [MCP guide](https://github.com/PatrickKoss/baukit/blob/v{{ context.template_version }}/docs/guide/mcp.md) covers the configuration keys, the Helm `mcp` values,
the OAuth flow, client setup, personal access tokens and authentication
policies. This page covers the steps inside this product.

The generated `services` builder supplies the product name, package version and
instructions. Set an optional display title with `Implementation::with_title`.
Use `McpServices::with_success_text_prefix` when successful text needs an
untrusted-data marker. Include the separator in the prefix. Structured content
and tool errors keep their original values.

The product MCP crate's `authentication_policy` function supplies the policy
argument to `baukit_mcp::router`. It returns `JwtOnlyPolicy`. Replace that
function to select another policy, as the guide describes.

## Add a tool

Set each tool's `annotations` to `ToolAnnotations::default()` to keep the
read/write defaults. Override hints with `read_only`, `destructive`, `idempotent`
and `open_world`. Use `ToolAnnotations::with_title` for a display title.
Without overrides, read-only tools are non-destructive, write tools are
destructive, and every tool is closed-world. Keep `destructive` on writes that
overwrite state or make an irreversible transition. Set `idempotent(true)` on a
write that has no further effect when repeated with the same arguments. Set
`open_world(true)` only when the tool reaches systems outside the product.
Review annotation changes in `mcp-tools.json` with schemas and scopes.

Add a service trait and implementation in the product's ports and services,
or beside the MCP adapter when only MCP consumes it. Tool logic takes domain
types and calls that service trait. The MCP adapter converts JSON arguments,
maps `Principal` to the product identity, and converts results to JSON.
The generated `ItemReadService` demonstrates this boundary. Its sample reads
the shared item catalog. Products with user-owned records must scope the
service query to the verified issuer and subject.

Add a `ScopedTool` to `ItemTools::definitions` with its input and output schemas,
read-only annotation, and required OAuth scopes. Add its dispatch in `call`.
The HTTP layer requires all declared scopes before execution. `tools/list`
omits tools the principal cannot call. Product authorization still belongs
in the service. Add the scope to Keycloak and test the service and HTTP call.

Export the contract and review its diff:

```sh
cargo run --manifest-path backend/Cargo.toml -p {{ context.app_name }}-mcp --bin mcp-tools > backend/mcp-tools.json
cargo test --manifest-path backend/Cargo.toml -p {{ context.app_name }}-mcp --test tool_drift
```

## Errors

Use `ToolError::new` for default errors or `with_structured_content` for a
product error envelope. Test structured errors against the product's output
schema. Use `text_only()` to preserve text-only `isError` results with the
existing output schema.

## Resources and prompts

The generated `services` function returns tools-only `McpServices` for the
router's third argument. Add optional resource and prompt services there.
`ItemTools::schema` and `tool_drift` also record empty resource and prompt
registries. When adding either service,
include its definitions in that export and review `mcp-tools.json`.

## Request cancellation

Tool calls receive a `baukit_mcp::CancellationToken` as their final argument.
The item adapter stops its pending service read when the client cancels or
disconnects. Use `run_until_cancelled()` or await `cancelled()` when adding
tools or resource reads. Cancellation notifications identify an active request
from the same issuer, subject and OAuth client.
