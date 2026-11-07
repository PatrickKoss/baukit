## Shared agent rules (baukit)

These rules come from baukit and `baukit check agent-docs` keeps this block in sync.
Edit `lint/agent-rules.md` in baukit, not this copy.

- Run the repo gate before you call a task done. Rust tests run with
  `-- --include-ignored`, so Docker-gated tests run too.
- Every suppression (`#[allow]`, `#[expect]`, `eslint-disable`, `ast-grep-ignore`)
  carries a reason that names the concrete invariant. `#[allow]` needs `reason = "..."`,
  the comment forms need `-- reason`. Suppression counts in `baukit-check.toml` only go down.
- Do not grow god files. `baukit check file-size` caps Rust files at 1000 lines and
  TS files at 500. The allowlist only shrinks: split a file instead of raising its entry.
- SQL on tenant data is owner-scoped. Put `(owner, id)` in the WHERE clause, never `id` alone.
- Outbound HTTP goes through baukit-egress. Do not build a raw `reqwest::Client`.
- Types that hold secrets never derive `Debug`. Write a redacting impl or use a secret wrapper.
- The threat model lives in `docs/threat-model.md`. The security skills in baukit's
  `agent-skills/` walk through it; use them when you touch auth, tenancy, or egress.
- Prefer the existing linters (clippy, eslint, ast-grep, cargo-deny). `baukit check`
  covers what they cannot: lint baselines, file sizes, suppression budgets, and this block.
