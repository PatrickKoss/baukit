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
