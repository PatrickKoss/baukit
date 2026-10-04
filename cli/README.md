# Baukit CLI

Install the CLI from the release tag:

```sh
cargo install --git https://github.com/PatrickKoss/baukit --tag v0.7.0 --locked baukit-cli
baukit --version
```

The CLI is not published to crates.io. Use the tag that matches the product's
`template_version` so its embedded templates and dependency versions agree.

`baukit new NAME ...` creates a new `NAME/` directory. To scaffold an existing
or orphan-branch repository root without overwriting differing files, use
`--dir . --into-existing`. Generation deliberately does not commit or push:

```sh
git add .
git commit -m "Scaffold product with Baukit"
git remote add origin git@github.com:YOUR_ORG/NAME.git
git push -u origin main
```

Use `--port-offset N` when several generated products run on one development
machine. The CLI adds `N` to each generated PostgreSQL, API, operations,
Keycloak, and fake-provider host port, then records the offset in `baukit.toml`.
An offset of zero keeps the default ports and is omitted from the manifest.
`baukit doctor` checks the generated port references against the recorded
offset.

`openapi.consumers` lists generated TypeScript declarations. Raw schema copies
remain product-owned until a second product needs them. The
[raw OpenAPI mirror design](../docs/platform/openapi-mirrors.md) records the
proposed manifest and strict-check behavior.

Each frontend can have its own `pnpm-workspace.yaml`. A product can instead
use one root workspace whose `packages` patterns include every enabled app
(`mobile`, `web`, and pnpm-managed `mcp`). Doctor checks inclusions and exclusions in those
patterns.

`capabilities.analytics` selects the mobile analytics adapter. It accepts
`"posthog"` or `"none"`. Generated manifests select `"posthog"`; older
manifests without this setting do not select an adapter. Set it to `"none"`
when the mobile app uses `NoopTransport`. Doctor requires
`@baukit/analytics-posthog-native`
only for `"posthog"`. The web template uses `NoopTransport` and has no required
PostHog adapter. Auth adapters follow `capabilities.auth`; the Expo SQLite
adapter remains required by the generated mobile record store.

Products with independently assigned ports can override the offset in
`baukit.toml`. Host ports apply to local URLs and `make dev`; container ports
apply to Compose targets and deployment values. For example:

```toml
[ports]
api = { host = 17001, container = 8080, service = "backend" }
ops = { host = 17002, container = 9090, service = "backend" }
postgres = { host = 17003, container = 5432 }
keycloak = { host = 17004, container = 8080 }
```

The other supported names are `redis` and `fake_provider`. `service` overrides
the Compose service name. Undeclared ports retain their offset defaults.
PostgreSQL, Keycloak, and Redis must keep container ports 5432, 8080, and 6379
for the generated images. Doctor checks literal Compose mappings and parameter
defaults against declarations, and checks loopback source URLs against host
ports. Runtime environment overrides remain product-owned.

Doctor does not require the template's guidance filenames. Products can write
API policy, fake-provider, sync-table, navigation, observability, budget, and
local-data-retention guidance under their own names. Generated links should
be updated when a document moves. Machine-read inputs retain their contracts:
`mcp/docs/tools.md` for `docs:check`, the declared OpenAPI schema and consumers,
and `docs/openapi-accepted-breaks.json` when the compatibility gate reads it.

MCP drift checks and tests read the exports in `mcp/src/tools/read.ts` and
`write.ts`, so those files stay required. Doctor accepts exported `READ_TOOLS`
and `WRITE_TOOLS` metadata, or `READ_TOOL_NAMES` and `WRITE_TOOL_NAMES` catalogs.
The shared helper `mcp/src/tools/registry.ts` is a template implementation
choice. A product can move its types and result helpers if it updates its own
imports and keeps its tool docs, route checks, and server tests passing.
