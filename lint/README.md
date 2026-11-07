# Pattern rules

`lint/ast-grep/` holds project-specific [ast-grep](https://ast-grep.github.io) rules for Rust and
TypeScript. They catch patterns that clippy and typescript-eslint do not: secrets in derived `Debug`,
raw `reqwest` clients, SQL built with `format!`, double casts, tokens in web storage and raw HTML
injection. baukit ships the same rules to every product.

| Rule | Severity | Flags |
|---|---|---|
| `rust-secret-debug` | error | `#[derive(Debug)]` on a type with a secret-named `String`, `Vec<u8>` or `&str` field |
| `rust-raw-http-client` | error | `reqwest::Client::new/builder`, `ClientBuilder::new`, `reqwest::get` outside `adapters/egress` |
| `rust-sql-format` | error | `format!` that builds SQL and interpolates anything but a SCREAMING_CASE constant |
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
| `suppressions` | Counts `#[allow]`, `#[expect]`, `eslint-disable` and `ast-grep-ignore` against `[suppressions.max]`. Flags `#[allow]` without `reason =` and comment suppressions without ` -- reason`. |
| `patterns` | Runs the ast-grep rules above with ast-grep 0.51.1, embedded in the CLI. `error` rules fail; `hint` rules print with `--verbose`. |
| `agent-docs` | `CLAUDE.md` holds exactly one managed block equal to `lint/agent-rules.md`, and `AGENTS.md` is a symlink to `CLAUDE.md`. |

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
```

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
