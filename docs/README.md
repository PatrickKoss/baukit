# baukit docs

Start with the [guide](guide/README.md) if you build a product with baukit. The rest of this folder holds the detail the guide links to.

## Guide

The [guide index](guide/README.md) also lists every crate and package.

- [Getting started](guide/getting-started.md)
- [Scaffolding](guide/scaffolding.md)
- [CLI](guide/cli.md)
- [Manifest](guide/manifest.md)
- [Backend](guide/backend.md)
- [Authentication](guide/auth.md)
- [Data and sync](guide/data-and-sync.md)
- [Jobs](guide/jobs.md)
- [Remote MCP](guide/mcp.md)
- [Suite and platform](guide/suite-and-platform.md)
- [Integrations](guide/integrations.md)
- [Push and notifications](guide/push-and-notifications.md)
- [Privacy](guide/privacy.md)
- [Outbound HTTP and limits](guide/outbound-and-limits.md)
- [Frontend](guide/frontend.md)
- [Observability](guide/observability.md)
- [Deploying](guide/deploying.md)
- [Local platform](guide/local-platform.md)
- [Quality gates](guide/quality-gates.md)
- [Security](guide/security.md)
- [Upgrading](guide/upgrading.md)
- [Agent skills](guide/agent-skills.md)

## Reference

- [Contracts](platform/) say what a product must do or can rely on. Start with the [compatibility matrix](platform/compatibility-matrix.md) and the [telemetry spec](platform/telemetry-spec.md).
- [Architecture decision records](adr/README.md) explain why baukit works the way it does today.
- [Migrations](migrations/README.md) list what a product changes for each breaking change.
- [Runbooks](runbooks/) say what to do when an alert fires, for [product services](runbooks/app/) and the [platform](runbooks/platform/).

## Working on baukit

- [Verification](verification.md) covers the gate, fuzzing, Kani, Verus, TLA+ and the shared vectors.
- [Releasing](releasing.md) is the release procedure.
- [Threat model](threat-model.md) lists every trust boundary and its controls.
