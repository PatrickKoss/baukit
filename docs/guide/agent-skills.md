# Give coding agents the baukit skills

baukit ships skills for Claude Code and Codex that carry its conventions: how to add an endpoint
that stays owner scoped, how to read a gate failure, how to upgrade. This page installs them into
a product and says which skill fits which task. At the end your agent loads the right skill when
the task comes up.

## Install

From a baukit checkout:

```sh
make install-skills TARGET=../my-product
```

Or call the installer directly from anywhere:

```sh
path/to/baukit/agent-skills/install.sh --target ../my-product [--claude] [--codex] [--copy]
```

Source: `agent-skills/install.sh`. The installer links each skill into `.claude/skills`,
`.agents/skills` or both. Without `--claude` or `--codex` it uses whichever of the two
directories exists in the target. `--copy` copies the files instead of linking, for a product that
does not keep a baukit checkout next to it. It refuses to overwrite a skill directory it did not
create.

Links follow the checkout, so pulling baukit updates the skills. Copies stay at the version you
installed until you run the installer again.

## Pick a skill

| Skill | Use it when |
| --- | --- |
| `baukit-new-product` | Creating a product with `baukit new`. |
| `baukit-upgrade` | Moving a product to a newer release train. |
| `baukit-add-endpoint` | Adding or changing a backend route, DTO or OpenAPI contract. |
| `baukit-add-integration` | Adding an external provider, OAuth connection, webhook or import worker. |
| `baukit-offline-sync` | Building outboxes, cursors, tombstones or multi-client sync. |
| `baukit-mirrored-domain-logic` | The same domain rule must run in Rust and TypeScript. |
| `baukit-localization` | Adding locales or fixing locale fallback. |
| `baukit-accessibility` | Building or reviewing accessible web and native screens. |
| `baukit-observability` | Wiring dashboards, alerts, recording rules or new metrics. |
| `baukit-quality-gates` | Choosing a quality profile or reading a gate failure. |
| `baukit-code-conformance` | `baukit check`, clippy or eslint fails and the code needs a fix, not a suppression. |
| `baukit-threat-model` | Starting a product or adding a route group, integration, webhook or data class. |
| `baukit-security-testing` | A threat-model row needs a test: fuzz, proptest, cross-tenant, SSRF or limits. |
| `qa` | Auditing a running app through its UI before a release. |

Each skill is `agent-skills/skills/<name>/SKILL.md`. Its front matter `description` is what the
agent matches against the task, so a precise description matters more than a long body.

## Agent rules in CLAUDE.md

Skills load on demand. The rules that always apply live in a managed block at the end of the
product's `CLAUDE.md`, copied from `lint/agent-rules.md`. `AGENTS.md` is a symlink to
`CLAUDE.md`, so Codex reads the same text. `baukit check agent-docs` fails when the block
drifts, and `baukit check agent-docs --fix` rewrites it.

## Add a skill

Create `agent-skills/skills/<name>/SKILL.md` with `name` and `description` front matter, and add
a row to the table in `agent-skills/README.md`. Keep each skill to one task, and point at code
and docs in the repo instead of copying them.

## Reference

- [`agent-skills/README.md`](../../agent-skills/README.md)
- [`lint/agent-rules.md`](../../lint/agent-rules.md)
- [The baukit CLI](cli.md#baukit-check)
- [ADR 0013: code rules enforced by baukit check](../adr/0013-code-rules-enforced-by-baukit-check.md)
