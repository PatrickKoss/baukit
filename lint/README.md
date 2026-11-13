# Pattern rules

`lint/ast-grep/` holds project-specific [ast-grep](https://ast-grep.github.io) rules for Rust and
TypeScript. They catch patterns that clippy and typescript-eslint do not: secrets in derived `Debug`,
raw `reqwest` clients, SQL built with `format!`, double casts, tokens in web storage and raw HTML
injection. baukit ships the same rules to every product.

| Rule | Severity | Flags |
|---|---|---|
| `rust-secret-debug` | error | `#[derive(Debug)]` on a type with a secret-named `String`, `Vec<u8>` or `&str` field |
| `rust-raw-http-client` | error | `reqwest::Client::new/builder`, `ClientBuilder::new`, `reqwest::get` outside `adapters/egress` |
| `rust-sql-format` | error | `format!` that builds SQL (statements and fragments such as predicates, `ORDER BY`, text with `$1`) and interpolates anything but a SCREAMING_CASE constant |
| `rust-sql-splice` | error | `replacen`, `split_once`, `rsplit_once`, `find` on a `" WHERE "` text |
| `rust-sql-include-replace` | error | `replace` or `replacen` with a slot or placeholder literal (`{x}`, `$1`, `__X__`) on SQL from `include_str!("*.sql")`, direct or let-bound |
| `rust-sql-assert-safe` | error | `AssertSqlSafe(...)` outside tests |
| `rust-sql-format-const` | error | `format!` with a captured `{IDENT}` and an uppercase `SELECT`, `FROM` or `WHERE` |
| `rust-sql-unscoped-by-id` | hint | SQL literal whose only predicate is `WHERE id = $1` |
| `ts-double-cast` | error | `as unknown as T` outside tests |
| `ts-token-web-storage` | error | token-like keys or values in `localStorage`/`sessionStorage`, token puts into IndexedDB |
| `ts-dangerous-html` | error | `dangerouslySetInnerHTML`, `innerHTML =`/`outerHTML =` with non-constant values |

Each rule's `message` says what to do instead and its `note` says why.

## Running

Install the pinned version (`mise install`, or `cargo install ast-grep --locked --version 0.51.1`), then:

```bash
ast-grep scan -c lint/ast-grep/sgconfig.yml rust/crates cli/src typescript/packages templates
ast-grep test -c lint/ast-grep/sgconfig.yml
```

`scan` exits non-zero when it finds an error-level match. Hints print but never fail the run.
`.ts` files are parsed with the TSX grammar (`languageGlobs` in `sgconfig.yml`), so one rule covers both.

## Suppressing one match

Put the ignore comment on the line above the match, and give a reason that names the concrete invariant:

```rust
// ast-grep-ignore: rust-sql-format -- table comes from the closed TABLES list of constants
let sql = format!("DELETE FROM {table} WHERE tenant_id = $1");
```

```ts
// ast-grep-ignore: ts-double-cast -- RN View ref is the DOM element on web (react-native-web)
const el = ref.current as unknown as HTMLElement;
```

Always name the rule id; a bare `// ast-grep-ignore` silences every rule. A suppression without a
reason after `--`, or with a vague one like "false positive", fails review.

## Adding a rule

Add `rules/<id>.yml` and `rule-tests/<id>-test.yml` with valid and invalid cases, then run
`ast-grep test -c lint/ast-grep/sgconfig.yml -U` once to write the snapshots under
`rule-tests/__snapshots__/` and review them. Do not add rules that clippy or typescript-eslint already
cover.

## Products

Products run the copy shipped with their baukit version, so every product checks the same rules.
Point `-c` at that copy and pass the product's source roots:

```bash
ast-grep scan -c <baukit>/lint/ast-grep/sgconfig.yml backend web mobile
```

The rules' `ignores` use `**/` globs, so they work from any root.

# `baukit check`

`baukit check [RULE...] [--root DIR]` enforces the shared rules in baukit and in every product gate.
With no rule it runs all of them. It prints one line per finding (`rule: path[:line]: message`), then a
summary line per rule. Exit code 0 means clean, 1 means findings, 2 means a usage or config error.
Lines marked `note:` never fail the run.

| Rule | Checks |
|---|---|
| `lints` | Every Cargo workspace root (or standalone package) has the baseline from `lint/rust/lints.toml` at the same or a stricter level, a `clippy.toml` next to it that covers `lint/rust/clippy.toml`, and members that set `[lints] workspace = true`. Every `eslint.config.{js,mjs,ts}` imports `@baukit/eslint-config` (baukit's own `typescript/` may import it by relative path). `cargo fuzz` crates are skipped. |
| `file-size` | Rust files stay at or under 1000 lines, TS/TSX files at or under 500. Tests, `*.test.*`, `*.spec.*`, `tests.rs`, `*_tests.rs`, `.d.ts` and generated files are exempt. |
| `suppressions` | Counts each suppression form against its `[suppressions.max]` key (see below). Flags a suppression without its reason, `@ts-ignore`, and every bulk suppression mechanism, including a nested ruff config that selects fewer rules than the root and a gitleaks allowlist with an unscoped pattern regex. Needs `ruff` for `# noqa` in `.py` files. |
| `patterns` | Runs the ast-grep rules above with ast-grep 0.51.1, embedded in the CLI. `error` rules fail; `hint` rules print with `--verbose`. |
| `agent-docs` | `CLAUDE.md` holds exactly one managed block equal to `lint/agent-rules.md`, and `AGENTS.md` is a symlink to `CLAUDE.md`. |
| `observability` | Dashboards, alerts and recording rules from `[observability]` in `baukit-check.toml`. See below. Without that table it prints a note and passes. |
| `template-files` | Every file in `.baukit/template-files.toml` still has the SHA-256 that `baukit new` recorded. `--update` rewrites the hashes after a deliberate upgrade. Without the manifest it prints a note and passes. |

`target/`, `node_modules/`, `vendor/`, `.generated-fixture/` and `.git/` are never scanned.

## `baukit-check.toml`

Optional, at the checked root. Limits may only go down; a higher value is a config error.

```toml
[file_size]
rust-max-lines = 1000
ts-max-lines = 500

# Files already over the limit, with their current line count.
[file_size.allowlist]
"src/legacy.rs" = 1240

[suppressions.max]
rust-allow = 3
rust-expect = 12
eslint-disable = 0
ast-grep-ignore = 1
ts-expect-error = 0
ts-ignore = 0
prettier-ignore = 0
noqa = 0
type-ignore = 0
nosec = 0
shellcheck-disable = 0
nolint = 0
```

Each key and the reason its form needs:

| Key | Form | Reason |
|---|---|---|
| `rust-allow` | `#[allow(...)]` | `reason = "..."`; prefer `#[expect]` |
| `rust-expect` | `#[expect(...)]` | `reason = "..."` (rustc enforces it) |
| `eslint-disable` | `eslint-disable*` comments | ` -- reason` |
| `ast-grep-ignore` | `ast-grep-ignore: rule` | ` -- reason` |
| `ts-expect-error` | `// @ts-expect-error` | ` -- reason` |
| `ts-ignore` | `// @ts-ignore` | always a finding; use `@ts-expect-error -- reason` |
| `prettier-ignore` | `// prettier-ignore`, and `<!-- prettier-ignore -->` or `<!-- prettier-ignore-start -->` in `.md`, `.mdx`, `.html`, `.vue`, `.svelte` (outside Markdown code fences) | a comment on the line above (prettier needs the directive alone) |
| `noqa` | `# noqa` | codes and a reason: `# noqa: S311 -- reason` |
| `type-ignore` | `# type: ignore` | codes and a trailing comment: `# type: ignore[attr-defined]  # reason` (mypy's grammar has no ` -- `) |
| `nosec` | Python `# nosec`, Go `// #nosec` | ` -- reason` |
| `shellcheck-disable` | `# shellcheck disable=` in `*.sh`, `*.bash` and extensionless files whose `#!` names sh, bash, zsh or dash | `# reason` after the directive, or a comment on the line above |
| `nolint` | Go `//nolint` | linters and a reason: `//nolint:errcheck // reason` |

Bulk suppression has no budget; each one is a finding. That covers `eslint-suppressions.json`, the eslint
`--suppress-all`, `--suppress-rule`, `--prune-suppressions` and `--suppressions-location` flags in a
`package.json` (and the file the last one names), clippy `-A`, `--allow` or `--cap-lints` after `clippy` or
in `RUSTFLAGS` in gate scripts (`*.sh`, `*.yml`, `Makefile`, `justfile`, extensionless shell scripts), a lint set to `allow` in a Cargo
`[lints]` or `[workspace.lints]` table, and `-A`, `--allow` or `--cap-lints` in `.cargo/config.toml`.

golangci configs (`.golangci.yml`, `.yaml`, `.toml`) may not set `disable-all: true` or
`linters.default: none`, list `linters.disable` entries, set `issues.new` or `new-from-*`, or use
`exclude`, `exclude-rules`, `exclude-files`, `skip-files`, or the v2 `exclusions.rules` and
`exclusions.presets`. `exclude-dirs`, `skip-dirs` and `exclusions.paths` may name only generated, build
or dependency output.

eslint configs (`eslint.config.*`) outside `@baukit/eslint-config` may not set a rule to `off`, `warn`, `0`
or `1`, or `reportUnusedDisableDirectives` below error. `ignores`, `ignorePatterns` and `.eslintignore`
may name only generated, build or dependency output: `dist`, `build`, `coverage`, `out`, `node_modules`,
`vendor`, `target`, `generated`, `.expo`, `.qa`, `.turbo`, `.next`, `.nuxt`, `.output`, `.svelte-kit` and
`storybook-static`.

Nested ruff configs (`ruff.toml`, `.ruff.toml`, or a `pyproject.toml` with `[tool.ruff]`, whichever ruff reads
for that directory) may not select fewer rules than the config at the root. A nested config replaces the root
unless it sets `extend`, so `select = ["F", "E9"]` below a root that selects `E`, `F` and `S` is a finding that
lists the dropped selectors. A nested `ignore` or `extend-ignore` entry that overlaps a root-selected rule the
root does not ignore is a finding too. `extend` the root and use `per-file-ignores` for exceptions. Without a
root ruff config there is nothing to compare and the check passes.

`.gitleaks.toml` allowlists (`[[allowlists]]`, the legacy `[allowlist]`, and the ones under `[[rules]]`) may not
carry a `regexes` entry that is anything but an exact value, `^value$` with every metacharacter escaped, unless
the same entry has a non-empty `paths` or `commits`. A pattern such as `^[a-z_]+$` or `EXPO_PUBLIC_KEY` without
a scope hides matches everywhere in the repository.

Fix the findings instead.

An unused `# noqa` is a `suppressions` finding with its file and line, and it does not count against the
budget. `baukit check suppressions` runs `ruff check --extend-select RUF100` with the project's own config
when a `.py` or `.pyi` file has a `# noqa`. `--extend-select` overrides `select`, `ignore` and `extend-ignore`
but not `per-file-ignores`, so an entry there that ignores `RUF100` is a finding too. Remove dead comments with
`ruff check --extend-select RUF100 --fix`. The run needs `ruff` on `PATH` and fails when it is missing. In the
baukit checkout `templates/` is skipped for this check, because the generated fixture lints it with the
template's own ruff config; its `# noqa` comments still count against the budget.

The allowlist and the maxima only shrink. A listed file that grows, drops under the limit, or no longer
exists is a finding, and so is a suppression count above its max. A missing max counts as 0. When a count
falls below its max the run prints `lower max to N`; `baukit check suppressions --update` writes the lower
values.

## Fixes

- `baukit check agent-docs --fix` writes or replaces the managed block (appending it when missing) and
  creates the `AGENTS.md` symlink. It refuses when `AGENTS.md` is a regular file that differs from
  `CLAUDE.md`.
- `baukit check suppressions --update` lowers `[suppressions.max]`; it never raises a value.

The Rust baseline lives in `lint/rust/`, the agent rules in `lint/agent-rules.md`. Both are compiled into
the CLI, so a product checks against the rules of the baukit version it pins.

## `observability`

```toml
[observability]
dashboards = ["deploy/observability/dashboards"]   # files, or directories of *.json
rules = ["deploy/observability/alerts"]             # files, or directories of *.yml and *.yaml
sources = ["backend"]                               # Rust files that emit metrics
metrics = ["deploy/observability/product-metrics.txt"]  # check-metric-names.py --allowlist format
datasource-uids = ["prometheus"]                    # fixed uids allowed besides datasource variables
metric-prefix = "shop_"                             # optional
```

The rule runs the expression checks of `deploy/observability/lint/check-metric-names.py` on every
dashboard `expr` and every rule `expr:`: no `http_requests_duration_seconds`, no `service_name` in
Loki selectors, no status classes like `"5xx"`, `job_kind` instead of `job` on worker metrics, and
every referenced metric must exist. A metric exists when it is in Baukit's catalog, a recording rule,
a `metrics` list, or a `counter!`, `gauge!` or `histogram!` call (or its `describe_` form) with a
literal name under `sources`. The Python script stays the source of truth for Baukit's CI;
`cli/tests/check_observability.rs` runs both on the same fixtures and on `deploy/observability`.

On top of that it checks what the product linters checked:

- counters end in `_total`, gauges and histograms don't, and product
  metrics start with `metric-prefix` when it is set;
- every emitted product metric appears on a dashboard;
- every dashboard `datasource` names a datasource variable declared in `templating` or `__inputs`,
  a uid from `datasource-uids`, or a Grafana built-in;
- a recording rule does not reuse the name of an emitted metric.

`promtool check rules` and `promtool test rules` still run in the product gate; they need Docker.

