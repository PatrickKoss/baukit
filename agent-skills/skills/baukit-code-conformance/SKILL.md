---
name: baukit-code-conformance
description: Fix findings from `baukit check`, clippy or eslint in baukit or a Baukit product without suppressing them. Use when `baukit check` (lints, file-size, suppressions, patterns, agent-docs, observability, template-files) fails, when `cargo clippy -- -D warnings` reports unwrap_used, expect_used, indexing_slicing, cast_*, too_many_lines, needless_pass_by_value or another strict lint, when eslint reports strict-boolean-expressions, max-lines-per-function, max-params, complexity or an eslint-comments rule, or when an ast-grep rule such as rust-sql-format or ts-double-cast fires.
---

# Fix lint and check findings

A finding means the code breaks a rule the whole fleet shares. Fix the code. A suppression is the last
resort, it needs a reason that names a concrete invariant, and it costs budget in `baukit-check.toml`.

Run the failing tool again after every fix:

```sh
baukit check --root .                     # all rules; `baukit check file-size` for one
cargo clippy --all-targets --all-features -- -D warnings
corepack pnpm lint
```

In baukit itself, run the CLI with `cargo run --manifest-path cli/Cargo.toml --bin baukit -- check --root .`.

## Changes that get rejected

Reviewers reject the whole change for any of these:

- `#[allow(...)]` without `reason = "..."`, or any suppression whose reason is vague ("needed", "false
  positive", "clippy is wrong") instead of naming the invariant.
- Lowering a threshold, deleting a lint, or deleting, `#[ignore]`-ing or weakening a test to get green.
- Swallowing the error that `unwrap`/`expect` used to surface: `unwrap_or_default()` where failure is a
  bug, `let _ =`, `.ok()` that drops an error, `if let Some(x) = ... {}` with no `else`.
- Moving code into `#[cfg(test)]` or a test file to dodge a production lint.
- Raising a number in `baukit-check.toml` without a listed suppression that justifies it.

## `baukit check` rules

Output is `rule: path[:line]: message`. Exit 1 means findings, 2 means a usage or config error.

**lints.** A Cargo workspace lacks the baseline from `lint/rust/lints.toml`, its `clippy.toml` does not
cover `lint/rust/clippy.toml`, a member misses `[lints] workspace = true`, or an `eslint.config.*` does not
import `@baukit/eslint-config`. Copy the missing entries at the same or a stricter level. Never relax a
baseline lint to `allow` in the workspace table.

**file-size.** A Rust file is over 1000 lines or a TS/TSX file over 500. Split it by responsibility (see
"Splitting a file" below). Tests and generated files are exempt.

**suppressions.** A count of `#[allow]`, `#[expect]`, `eslint-disable`, `ast-grep-ignore`,
`@ts-expect-error`, `prettier-ignore`, `# noqa`, `# type: ignore`, `# nosec`, `# shellcheck disable=` or
Go `//nolint` exceeds its `[suppressions.max]` key, or a suppression lacks its reason (forms in
`lint/README.md`). `@ts-ignore` and bulk suppression are always findings: `eslint-suppressions.json`,
`eslint --suppress-all`, clippy `-A` or `--cap-lints` in a gate, a lint at `allow` in `[lints]` or
rustflags, an eslint rule at `off`/`warn` or an ignore of source outside `@baukit/eslint-config`, and
golangci `disable`/`exclude` settings, a nested ruff config whose `select` drops rules the root selects
(`extend` the root and use `per-file-ignores`), and a `.gitleaks.toml` allowlist whose pattern `regexes`
have no `paths` or `commits` (scope the entry, or anchor the exact value as `^value$`). An unused `# noqa` is a finding with file:line;
remove it with `ruff check --extend-select RUF100 --fix`, and do not ignore `RUF100` in `per-file-ignores`. Remove the suppression by fixing the code. If the
count drops, lower the max (see "Lowering the budget").

**patterns.** An ast-grep rule from `lint/ast-grep/rules/` matched. Each rule and its fix:

- `rust-secret-debug`: a type with a secret-named field derives `Debug`. Wrap the field in
  `secrecy::SecretString` or write a `Debug` impl that prints `"[redacted]"`.
- `rust-raw-http-client`: `reqwest::Client::new/builder`, `ClientBuilder::new` or `reqwest::get` outside
  `adapters/egress`. Send the request through `baukit_egress::GuardedClient`.
- `rust-sql-format`: `format!` builds SQL from a value. Bind it with `$1` and `.bind(value)`. Only
  SCREAMING_CASE constants may be interpolated.
- `rust-sql-unscoped-by-id` (hint): `WHERE id = $1` alone. Add the owner: `WHERE owner_id = $1 AND id = $2`,
  and add a test where a second principal gets 404.
- `ts-double-cast`: `as unknown as T` outside tests. Parse with a guard from `@baukit/type-guards`.
- `ts-token-web-storage`: a token goes into `localStorage`, `sessionStorage` or IndexedDB. Keep it in
  memory or in the auth package's broker.
- `ts-dangerous-html`: `dangerouslySetInnerHTML` or `innerHTML =` with a non-constant. Render text nodes.

**agent-docs.** `CLAUDE.md` does not hold exactly one managed block equal to `lint/agent-rules.md`, or
`AGENTS.md` is not a symlink to it. Run `baukit check agent-docs --fix`. Edit the rules in baukit, not in
the product.

**observability.** A dashboard, alert or recording rule names a metric outside the Baukit and product
vocabularies. Register the product metric or fix the query.

**template-files.** A template-owned file differs from the hash manifest. Revert it and put the product
change in the extension hook the template provides.

## Strict clippy lints

The baseline is `lint/rust/lints.toml` plus `lint/rust/clippy.toml`. CI runs with `-D warnings`, so every
`warn` fails. Tests may `unwrap`, `expect`, index, panic and print (`allow-*-in-tests`).

### `unwrap_used`, `expect_used`, `panic`, `missing_panics_doc`

Production code returns the error or makes the bad state unrepresentable.

```rust
// before
let port: u16 = env::var("PORT").unwrap().parse().expect("port");
// after
let raw = env::var("PORT").map_err(|_| ConfigError::Missing("PORT"))?;
let port: u16 = raw.parse().map_err(|source| ConfigError::Invalid { key: "PORT", source })?;
```

When the value is known present, show that with a pattern, not an `expect`:

```rust
let Some((first, rest)) = segments.split_first() else {
    return Err(PathError::Empty);
};
```

`missing_panics_doc` disappears once the function stops panicking. Do not add a `# Panics` section to
keep a panic in production code.

Legitimate `#[expect]`: a value built from constants the compiler cannot see through.

```rust
#[expect(clippy::expect_used, reason = "DEFAULT_RETRY is a literal parsed in a unit test; it cannot fail at runtime")]
static RETRY: LazyLock<Duration> = LazyLock::new(|| parse_duration(DEFAULT_RETRY).expect("valid literal"));
```

Prefer a `const` constructor or a unit test on the literal over this.

### `indexing_slicing`

`v[0]` and `&s[a..b]` panic on bad input. Use `get`, `first`, `split_first`, `split_at_checked`,
`chunks_exact` or iterators.

```rust
// before
let kind = parts[0];
// after
let Some(kind) = parts.first() else { return Err(ParseError::MissingKind) };
```

### `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap`, `cast_precision_loss`

Use `try_from` and map the error, or widen instead of narrowing.

```rust
// before
let limit = query.limit as u32;
// after
let limit = u32::try_from(query.limit).map_err(|_| ApiError::invalid("limit"))?;
```

For `u64 -> f64` metrics, keep the value in an integer type until the exporter, or use
`f64::from(u32)` where the source fits. A reasoned `#[expect(clippy::cast_precision_loss, reason =
"byte counts stay below 2^53, the f64 mantissa")]` is legitimate when the bound is real and stated.

### `too_many_lines` (80 lines)

Extract named steps. A handler that parses, authorizes, loads and maps becomes four functions. Long tests
follow the same rule: split them into named scenario helpers that each keep their assertions. Neither
production code nor tests may suppress the lint.

### `needless_pass_by_value`

Take `&T` or `&str` when the function only reads the argument. Keep ownership when you store or move
it; then the lint does not fire. Axum extractors are owned by design: if the lint fires on a handler,
destructure the extractor in the signature (`State(app): State<AppState>`) and pass `&app` down.

### `manual_let_else`

Replace `let x = match opt { Some(x) => x, None => return ... };` with `let Some(x) = opt else { return ... };`.

### `struct_excessive_bools`, `fn_params_excessive_bools`

Replace flag sets with an enum or a small options struct of enums. `fn render(compact: bool, dark: bool,
rtl: bool)` becomes `fn render(opts: RenderOptions)` with `Density` and `Theme` enums.

### `unused_async`, `return_self_not_must_use`

Drop `async` from a function that never awaits. Add `#[must_use]` to builder methods that return `Self`.

### `print_stdout`, `print_stderr`, `dbg_macro`, `todo`, `unimplemented`

Log with `tracing`. CLIs write through an injected `Write` so tests can capture output. Finish the code
instead of leaving `todo!()`.

### `allow_attributes_without_reason`, `unsafe_code`

Every `#[allow]` needs `reason`, and baukit wants `#[expect(lint, reason = "...")]` instead: `expect`
fails once the lint stops firing, so stale suppressions cannot pile up. `unsafe_code` is denied.

### `clippy.toml` disallowed methods

`std::thread::sleep` blocks the runtime: use `tokio::time::sleep` or a notify-based wait. The `reqwest`
constructors skip the SSRF guard: use `baukit_egress::GuardedClient`.

## Strict eslint rules

`@baukit/eslint-config` adds these to typescript-eslint `strictTypeChecked` and `stylisticTypeChecked`.
Examples are in [references/typescript.md](references/typescript.md).

- `max-lines-per-function` (80), `max-lines` (500), `max-depth` (4), `max-params` (4), `complexity` (15):
  extract helpers, use early returns, pass an options object. Test files relax the size and param limits.
- `@typescript-eslint/strict-boolean-expressions`: compare explicitly (`value !== undefined`,
  `items.length > 0`, `text !== ''`).
- `@typescript-eslint/consistent-type-assertions`: no object literal assertions (`{} as Foo`). Annotate the
  variable (`const foo: Foo = {...}`) or use `satisfies`.
- `@typescript-eslint/explicit-module-boundary-types`: exported functions declare their return type.
- `@typescript-eslint/no-unused-vars`: delete it, or prefix an unused parameter with `_`.
- `@typescript-eslint/no-explicit-any`, `no-unsafe-*`, `no-non-null-assertion`: parse unknown data with a
  `@baukit/type-guards` guard and narrow instead of asserting.
- `@eslint-community/eslint-comments/require-description` and `no-unlimited-disable`: every disable
  names one rule and ends with `-- reason`. `reportUnusedDisableDirectives` fails stale disables.

A legitimate disable names the invariant:

```ts
// eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- the map is seeded with every Locale key at module load
const catalog = catalogs.get(locale)!;
```

## Splitting a file

An allowlisted file in `[file_size.allowlist]` may not grow at all. Any change that adds lines to it must
split it first.

1. Find the seams: a group of functions around one type, a submodule of private helpers, a large `impl`
   for one trait, tests at the bottom.
2. Move tests out first: `#[cfg(test)] mod tests;` with the body in `tests.rs` or `<name>_tests.rs`
   (exempt from the size limit). In TS, move cases to `*.test.ts`.
3. Turn `foo.rs` into `foo/mod.rs` plus `foo/<concern>.rs`. Re-export what callers use (`pub use
   concern::Thing;`) so the public API stays the same. In TS, split into sibling modules and keep
   `index.ts` exports unchanged.
4. Do not split by line number into `part1.rs`/`part2.rs`. Each new file names its responsibility.
5. Update or remove the allowlist entry: once the file is under the limit, delete the entry; if it is
   still over, set the entry to the new, lower count. A stale entry is itself a finding.

## Lowering the budget

When your fix removes suppressions, `baukit check suppressions` prints `lower max to N`. Run
`baukit check suppressions --update` and commit the lower `baukit-check.toml`. The command never raises a
value. If you must add a suppression, raise its max by exactly one in the same commit and list the
suppression, its file:line and its reason in the change description.
