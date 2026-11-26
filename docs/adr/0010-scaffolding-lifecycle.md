# ADR 0010: The CLI scaffolds products from a manifest, re-renders without overwriting, and is held to golden trees

## Status
Accepted, 2026-11-27.

## Context

Products are generated once and then live for years. Templates keep changing, so a product needs a
way to take new template files without losing its own edits. Template changes also need review: a
small edit can change hundreds of generated files across many flavors.

## Decision

- `baukit new <name>` generates a product. Flags pick the parts: `--backend`, `--worker`,
  `--mobile`, `--web`, `--pwa`, `--mcp`, `--platform`, `--auth`, `--quality standard|strict` and
  `--port-offset`. `--dir` and `--into-existing` choose the target, and `--baukit-path` uses local
  crates.
- `baukit.toml` records the choices: `schema_version`, `template_version`, ports, `app`, `quality`,
  `gate`, `auth`, `capabilities`, `platform`, `dependencies`, `openapi`, `doctor`, `backend`,
  `gitops` and `postgres`.
- Re-running with `--force` adds missing files and never overwrites one that differs. It writes the
  conflicts to `baukit-conflicts.txt` for the product to merge.
- `.baukit/template-files.toml` stores a hash of each template-owned file.
- `baukit doctor` checks the host and the product against the manifest. `baukit generate` renders
  single outputs: `dockerfile` (with `--check`), `openapi-client`, `gitops` and
  `release-age-excludes`. `baukit check` is [ADR 0013](0013-code-rules-enforced-by-baukit-check.md).
- `cli/tests/snapshots/*.tree` pins a SHA-256 per generated file for every flavor.
  `cargo run --manifest-path cli/Cargo.toml --example bless_snapshots` regenerates them.
- `scripts/generated-fixture-gate.sh` generates each flavor and runs its full checks.

## Enforced by

- `cli/tests/generator.rs`: the golden tree tests, `force_reports_conflicts_without_overwriting` and
  `generation_can_render_directly_into_an_existing_repository_root`.
- The `template-files` rule of `baukit check` fails when a template-owned file differs from its hash.
- `cli/tests/template_migrations.rs`: `auth_template_copies_every_browser_session_migration`.
- Gate steps `fixture-1`, `fixture-2`, `affected-fixture` and `mcp-fixture` build and test the
  generated products.

## Consequences

A reviewer sees a template change as a diff of tree files and checks that each changed path was
meant to change. A product upgrade is a re-render plus a manual merge of the listed conflicts, so
heavily edited template files take more work. Fixture gates are the slowest part of the baukit gate.

## History

- 2026-08-25, commit `af6e2b4f` (baukit 0.1.0): `new`, the manifest and the golden trees.
- 2026-11-07, commit `860f6643`: the generator split into modules.
- 2026-11-07, commit `68649dce`: template file hashes and the `template-files` rule.
