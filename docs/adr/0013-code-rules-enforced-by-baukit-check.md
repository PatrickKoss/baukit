# ADR 0013: Code rules live in baukit check, and products inherit the same rules

## Status
Accepted, 2026-11-27.

## Context

Rules in a README drift. Agents and people both skip rules nobody checks, and per-repo linter
configs get loosened one suppression at a time. Products generated from baukit need the same
rules as baukit itself, or the templates would ship code that baukit's own gate rejects.

## Decision

`baukit check` in `cli/src/check/` runs these rules, listed by `all_rules` in `mod.rs`:

- `lints`: crates set clippy levels at least as strict as `lint/rust/lints.toml`. That covers
  `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, the lossy cast lints and
  `allow_attributes_without_reason`, which `-D warnings` turns into errors. Members inherit them with
  `[lints] workspace = true`. `clippy.toml` sets `too-many-lines-threshold` to 80 or less and lists
  `disallowed-methods`. ESLint configs import `@baukit/eslint-config`.
- `file-size`: Rust files stay at 1000 lines or less, TypeScript at 500. The allowlist in
  `baukit-check.toml` may not grow.
- `suppressions`: every suppression names a reason, `@ts-ignore` is forbidden, and the counts in
  `[suppressions.max]` only go down. It rejects bulk suppression: `eslint-suppressions.json`,
  `--suppress-all`, clippy `-A` or `--cap-lints` in gate scripts, lints set to `allow`, ESLint rules
  turned off outside `@baukit/eslint-config`, and golangci `disable` settings.
- `patterns`: ast-grep rules in `lint/ast-grep/rules/`, for example `rust-raw-http-client`,
  `rust-secret-debug`, `rust-sql-unscoped-by-id`, `ts-double-cast` and `ts-token-web-storage`.
- `agent-docs`: the shared block in `CLAUDE.md` equals `lint/agent-rules.md`, and `AGENTS.md` is a
  symlink to it.
- `observability`: dashboards, alerts and recording rules use only catalog metric names.
- `template-files`: template-owned files match their hashes.
- `fuzz-lockfile`: a strict product does not commit `backend/fuzz/Cargo.lock`.
- `security`: `axum::serve` gets connect info, bin crate merges sit inside the rate limiter, the ops
  service is `ClusterIP`, and CORS origins are https outside local config and never `null`.

`lint/agent-rules.md` is the prose version that agents read. Not every bullet has a rule:

- "Done means the gate passed" is checked by the gate, not by `baukit check`.
- Domain code free of frameworks is a template shape ([ADR 0009](0009-hexagonal-generated-backend.md)).
- The second principal test per id route has `baukit_test::check_cross_tenant_denied`, but nothing
  requires it.
- `cargo deny`, `osv-scanner` and actions pinned by SHA are gate steps, not `baukit check` rules.
- Threat model upkeep and "never log a token" have no check.

The `observability`, `template-files`, `fuzz-lockfile` and the ops and CORS parts of `security` have
no bullet in `lint/agent-rules.md`.

## Enforced by

- Gate step `baukit-check` runs `baukit check --root .` on baukit itself.
- Generated products run the same command in their gate.
- `cli/tests/check.rs`, `check_observability.rs` and `check_template_files.rs` test each rule
  against failing inputs.

## Consequences

A rule exists once, in code, and baukit and every product run the same version of it. Raising a
suppression budget needs a change to `baukit-check.toml` that a reviewer sees. Rules without a check
remain in the prose list and can drift.

## History

- 2026-11-07, commit `62a42572`: `baukit check` with `lints`, `file-size`, `suppressions`,
  `patterns` and `agent-docs`.
- 2026-11-07, commit `68649dce`: `observability` and `template-files`.
- 2026-11-08, commit `185ce4b6`: bulk suppression forbidden.
- 2026-11-19, commit `9e4171e1`: `fuzz-lockfile`.
- 2026-11-25, commit `a8dad46e`: `security`.
