# Agent skills

Skills that teach Claude Code and Codex how to work in a Baukit product. Install all of them into a
product with:

```sh
make install-skills TARGET=<product-dir>
# or, from anywhere:
<baukit-repo>/agent-skills/install.sh --target <product-dir> [--claude] [--codex] [--copy]
```

The installer links each skill into `.claude/skills` and `.agents/skills` (whichever exists, or the ones
you pass). `--copy` copies instead of linking. It refuses to overwrite a skill directory it did not
create.

| Skill | Use it when |
|---|---|
| [`baukit-new-product`](skills/baukit-new-product/SKILL.md) | creating a product repository with `baukit new` |
| [`baukit-upgrade`](skills/baukit-upgrade/SKILL.md) | moving a product to a newer Baukit release train |
| [`baukit-add-endpoint`](skills/baukit-add-endpoint/SKILL.md) | adding or changing a backend route, DTO or OpenAPI contract |
| [`baukit-add-integration`](skills/baukit-add-integration/SKILL.md) | adding an external provider, OAuth connection, webhook or import worker |
| [`baukit-offline-sync`](skills/baukit-offline-sync/SKILL.md) | building outboxes, cursors, tombstones or multi-client sync |
| [`baukit-mirrored-domain-logic`](skills/baukit-mirrored-domain-logic/SKILL.md) | the same domain rule must run in Rust and TypeScript |
| [`baukit-localization`](skills/baukit-localization/SKILL.md) | adding locales, translating catalogs or fixing locale fallback |
| [`baukit-accessibility`](skills/baukit-accessibility/SKILL.md) | building or reviewing accessible web and native screens |
| [`baukit-observability`](skills/baukit-observability/SKILL.md) | wiring dashboards, alerts, recording rules or new metrics |
| [`baukit-quality-gates`](skills/baukit-quality-gates/SKILL.md) | choosing a quality profile or reading a quality-gate failure |
| [`baukit-code-conformance`](skills/baukit-code-conformance/SKILL.md) | `baukit check`, clippy or eslint fails and the code needs fixing, not suppressing |
| [`baukit-threat-model`](skills/baukit-threat-model/SKILL.md) | starting a product or adding a route group, integration, webhook or data class |
| [`baukit-security-testing`](skills/baukit-security-testing/SKILL.md) | a threat-model row needs a test: fuzz, proptest, cross-tenant, SSRF or limits |
| [`qa`](skills/qa/SKILL.md) | auditing a running app through its UI before a release |

Every skill lives in `skills/<name>/SKILL.md`. Add a row here when you add one.
