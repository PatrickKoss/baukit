# E1: baukit Rust code quality audit

Scope: `rust/` workspace (21 crates), `cli/` workspace, Rust in `templates/`. Read-only audit, no repo files changed. Measured on main at 46937d9d, 2026-11-07.

## How the numbers were produced

All clippy counts come from one script run from the repo root, then a dedup counter (unique `(lint, file, line)` per primary span). Scratch dir is `$S`.

```bash
# $S/run.sh  (bash, not zsh: $L must word-split)
L="-W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -W clippy::indexing_slicing -W clippy::print_stdout -W clippy::print_stderr -W clippy::allow_attributes_without_reason -W clippy::dbg_macro -W clippy::todo -W clippy::unimplemented -W clippy::cognitive_complexity -W clippy::large_enum_variant"
export CARGO_BUILD_JOBS=4
cargo clippy --manifest-path rust/Cargo.toml --workspace --lib --bins --message-format=json -- $L > $1/rust-lib.json 2>&1
cargo clippy --manifest-path cli/Cargo.toml --bins --message-format=json -- $L > $1/cli.json 2>&1
```

```python
# $S/count.py <file.json>
import json,sys,collections
c=collections.Counter();seen=set()
for line in open(sys.argv[1]):
    try: m=json.loads(line)
    except: continue
    if m.get('reason')!='compiler-message': continue
    msg=m['message']; code=(msg.get('code') or {}).get('code')
    if not code or msg['level']!='warning': continue
    sp=[s for s in msg['spans'] if s['is_primary']]
    k=(code,sp[0]['file_name'],sp[0]['line_start']) if sp else (code,)
    if k in seen: continue
    seen.add(k); c[code]+=1
for k,v in c.most_common(): print(v,k)
```

Scope is `--lib --bins` on purpose: the counts are for production code. Tests are excluded. Note `cli --bins` also compiles the `baukit-cli` lib target as a dependency, so `cli/src/lib.rs` hits appear. `rust/` paths below are relative to `rust/` in the JSON, I prefixed them here.

Length and complexity used a lowered threshold through a scratch clippy.toml (no repo file touched):

```bash
echo 'too-many-lines-threshold = 80' > $S/conf/clippy.toml
CLIPPY_CONF_DIR=$S/conf cargo clippy --manifest-path rust/Cargo.toml --workspace --lib --bins --message-format=json -- -W clippy::too_many_lines -W clippy::cognitive_complexity
CLIPPY_CONF_DIR=$S/conf cargo clippy --manifest-path cli/Cargo.toml --lib --bins --message-format=json -- -W clippy::too_many_lines -W clippy::cognitive_complexity
```

### Raw totals (production code, strict set)

rust/ workspace, 683 warnings:

| count | lint |
|---|---|
| 249 | missing_errors_doc |
| 146 | must_use_candidate |
| 65 | doc_markdown |
| 42 | expect_used (12 in build.rs, 15 in baukit-test, 15 in other crates) |
| 26 | indexing_slicing (11 in baukit-test, 15 in other crates) |
| 22 | return_self_not_must_use |
| 19 | panic (17 in baukit-test, 2 in baukit-openapi, all intentional assert helpers) |
| 9 | ignored_unit_patterns |
| 7 | missing_panics_doc |
| 7 | unused_async_trait_impl |
| 6 | elidable_lifetime_names |
| 5 each | needless_pass_by_value, single_match_else, cast_possible_truncation, duration_suboptimal_units, allow_attributes_without_reason |
| 4 each | manual_let_else, semicolon_if_nothing_returned, needless_raw_string_hashes, cast_precision_loss, too_many_lines (default 100) |
| 3 each | assigning_clones, manual_is_variant_and, cognitive_complexity, cast_sign_loss, cast_possible_wrap, default_trait_access |
| 2 each | ref_option, map_unwrap_or, trivially_copy_pass_by_ref, unnested_or_patterns, unused_self, format_collect |
| 1 each | struct_excessive_bools, float_cmp, unused_async, redundant_closure_for_method_calls, match_wildcard_for_single_variants, missing_fields_in_debug, ip_constant, items_after_statements, match_same_arms, unchecked_time_subtraction |
| 0 | unwrap_used, dbg_macro, todo, unimplemented, print_stdout, print_stderr, large_enum_variant |

cli/, 59 warnings:

| count | lint |
|---|---|
| 16 | indexing_slicing |
| 6 | missing_errors_doc |
| 6 | print_stdout |
| 5 | too_many_lines (default 100) |
| 4 each | must_use_candidate, expect_used, struct_excessive_bools |
| 3 each | format_push_string, case_sensitive_file_extension_comparisons |
| 2 each | doc_markdown, cognitive_complexity, uninlined_format_args |
| 1 each | single_match_else, print_stderr |

## Already enforced

Do not propose these again.

- `rust/Cargo.toml:100-107`, `cli/Cargo.toml:14-21` and `templates/backend/backend/Cargo.toml.jinja:44-51` carry the same set: `unsafe_code = "deny"`, `clippy::all` warn, `dbg_macro`, `todo`, `unwrap_used`. All 21 crates in `rust/crates` opt in with `[lints] workspace = true` (`grep -rln lints rust/crates/*/Cargo.toml | wc -l` gives 21), and all 8 template crate manifests do too.
- CI runs `cargo clippy --all-targets -- -D warnings` for both workspaces (`Makefile:18`, `Makefile:53`, lanes `rust-clippy`/`cli-clippy` in `scripts/ci-gate.sh:30,36`) and on the generated fixture. So every `clippy::all` lint (correctness, suspicious, style, complexity, perf), including `await_holding_lock`, `large_enum_variant` and `needless_pass_by_value`-free defaults, is already a hard error.
- Production code has 0 `unwrap_used`, 0 `dbg_macro`, 0 `todo` hits. The existing rule works.
- `rustfmt.toml` sets `edition = "2024"`, CI checks fmt.
- `cargo deny ... check advisories licenses` runs in CI (`scripts/ci-gate.sh:64`).
- Global state: the only non-test statics are `LazyLock<Regex>` (`rust/crates/baukit-telemetry/src/lib.rs:125-133`, `rust/crates/baukit-test/src/metrics.rs:22-30`) and one cached catalog (`rust/crates/baukit-suite/src/platform/events.rs:14`). No mutable global config. Nothing to fix.
- `anyhow` appears in no library crate (`grep -n anyhow rust/crates/*/Cargo.toml` is empty). Only the CLI binary uses it, which is fine.
- No `Box<dyn Error>` in library public APIs. All 326 hits (`grep -rn --include="*.rs" -E "Box<dyn (std::error::)?Error" rust/crates | wc -l`) sit in `tests/` support code.
- No stringly typed public errors: `grep -rnE --include="*.rs" "^\s*pub (async )?fn .*Result<[^>]*, String>" rust/crates/*/src | wc -l` gives 0.
- No hexagonal leak found in `baukit-suite/src/domain` (`grep -rn "sqlx\|axum" rust/crates/baukit-suite/src/domain/*.rs | grep -v tests.rs` is empty).

## Findings, ranked by impact

### E1-01 cargo-deny `bans` and `sources` are configured but never run, and would fail today (high)

`rust/deny.toml:26-32` sets `wildcards = "deny"`, `unknown-registry = "deny"`, `unknown-git = "deny"`. CI only runs `check advisories licenses` (`scripts/ci-gate.sh:64`), so these rules are dead config. Running them fails:

```
rust/crates/baukit-auth/Cargo.toml:41
baukit-test = { path = "../baukit-test", features = ["sqlx-postgres"] }   # error[wildcard]
```

Prevalence: 12 crates with wildcard errors (path dev-deps without version), 15 duplicate-version warnings (base64, sha2, digest, rand, getrandom x3, hashbrown x3, tower-http, syn, ...). Command: `cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check bans sources 2>&1 | grep -E "^(warning|error)\[" | sort | uniq -c` (exit 2). `cli/` has no deny config and no deny step at all (`ls cli/deny.toml` fails).

Prevention: existing tool. Add `allow-wildcard-paths = true` under `[bans]`, switch CI to `cargo deny ... check` (all four checks), add the same step for `cli/Cargo.toml`. Fix cost: 1 config line, then 0 errors.

### E1-02 CLI god module: `cli/src/lib.rs` is 3572 lines with the longest, most complex functions in the repo (high)

```
cli/src/lib.rs:1347  fn doctor_with_host(root: &Path, host: &dyn DoctorHost) -> Result<Vec<String>> {   // ends 1634, cognitive complexity 46/25
cli/src/lib.rs:1755  fn validate_port_configuration(                                                     // ends 1990, 229 lines, complexity 30/25
```

Prevalence: largest .rs file in the repo (`find rust/crates cli/src -name '*.rs' -not -path '*/tests/*' | xargs wc -l | sort -rn | sed -n 2,11p`). Next are `rust/crates/baukit-auth/src/api_token.rs` 1961, `rust/crates/baukit-auth/src/verifier.rs` 1678, `cli/src/dockerfile.rs` 1238. Over 80 lines: 11 CLI functions, 8 in `rust/` (lowered-threshold command above). Top CLI: `lib.rs:1755` 229, `lib.rs:694` 154, `lib.rs:433` 149, `doctor_layout.rs:791` 144, `lib.rs:876` 128. Top rust: `baukit-mcp/src/security.rs:138` 127, `baukit-data-subject/src/server/worker.rs:53` 121, `baukit-jobs/src/runner.rs:263` 111. Clippy's count skips blank and comment lines, so `doctor_with_host` (288 physical lines) only shows up through cognitive complexity.

Prevention: `clippy::too_many_lines` (with `too-many-lines-threshold = 80` in a new `clippy.toml`) and `clippy::cognitive_complexity`. File size has no clippy lint: custom check needed, "fail if a non-test `.rs` file under `src/` exceeds 1500 lines unless listed in an allowlist that only shrinks". Fix cost: 4 + 5 at threshold 100, 8 + 11 at 80; 3 + 2 cognitive_complexity.

### E1-03 High cognitive complexity in runtime-critical async code (high)

```
rust/crates/baukit-runtime/src/lib.rs:504   async fn supervise_task<F, Fut>(        // complexity 52/25
rust/crates/baukit-mcp/src/security.rs:138  async fn authorize_inner(state: &Security, mut request: Request, next: Next) -> Response {  // 32/25
rust/crates/baukit-data-subject/src/server/worker.rs:53  async fn execute(   // 28/25
```

These are the task supervisor every product runs, the MCP auth path and the erasure worker. That is where a missed branch costs most. Prevalence: 3 in rust/, 2 in cli/ (`cognitive_complexity` rows in the totals above).

Prevention: `clippy::cognitive_complexity` (nursery-adjacent but stable enough at default 25). Fix cost: 5.

### E1-04 Unchecked slice indexing in library and CLI parsing code (med)

```
rust/crates/baukit-mcp/src/security.rs:101       || !hosts[0].to_str().ok().is_some_and(|host| {
rust/crates/baukit-egress/src/client.rs:500      body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
cli/src/dockerfile.rs:163                         && value.as_bytes()[0].is_ascii_alphanumeric(),
```

Most are guarded by a prior length check, but the guard lives one line away and survives refactors badly. Security-relevant files (mcp security, egress, credential-vault `cipher.rs:237`, auth `api_token.rs:609`) are in the list. Prevalence: 15 in rust/ non-test crates, 11 in baukit-test, 16 in cli/ (totals above).

Prevention: `clippy::indexing_slicing`. Fix cost: 42 (production) using `.get()`, `.first()`, `split_first`, `let [only] = slice else`.

### E1-05 `expect` in production library paths, undocumented panics (med)

```
rust/crates/baukit-runtime/src/lib.rs:297         hooks.push(hook.take().expect("drain hook is available"));
rust/crates/baukit-data-subject/src/server/router.rs:96   Ok(client.expect("checked client identity").to_owned())
rust/crates/baukit-http/src/precondition.rs:157   .expect("a validated prefix and a decimal revision form a valid header value")
```

`unwrap_used` is enforced but `expect_used` is not, so the codebase moved its panics to `expect`. Most are true invariants (header values from integers, `Default` impls of validated config like `rust/crates/baukit-ratelimit/src/memory.rs:212`). `router.rs:96` and `runtime/src/lib.rs:297` are control-flow invariants that a restructure (`let ... else`, `Option` pattern) would remove. `missing_panics_doc` fires 7 times, so public fns panic without a `# Panics` section. Prevalence: 15 expect in non-test, non-build.rs code of rust/, 4 in cli/; 7 missing_panics_doc.

Prevention: `clippy::expect_used` plus `allow-expect-in-tests = true` in clippy.toml, and `clippy::missing_panics_doc`. Each remaining invariant gets a `#[expect(clippy::expect_used, reason = "...")]`. Fix cost: 19 + 7 (12 build.rs hits need a crate-level `#![expect]` in the 3 build scripts).

### E1-06 Three copies of the lint config with no drift check (med)

```
rust/Cargo.toml:103          [workspace.lints.clippy]
cli/Cargo.toml:17            [lints.clippy]
templates/backend/backend/Cargo.toml.jinja:47   [workspace.lints.clippy]
```

Today the three tables are identical. Any addition from this report must land in three places, and the template copy ships to 12 product repos. `cli/` is a separate single-crate workspace so it cannot inherit. Prevalence: 3 copies (`grep -n "lints.clippy" rust/Cargo.toml cli/Cargo.toml templates/backend/backend/Cargo.toml.jinja`).

Prevention: custom check needed, "parse the three lint tables (and the shared clippy.toml if added) and fail when they differ", fits into `scripts/check-version-coherence.py` style. Alternatively make `cli/` a member of `rust/`'s workspace.

### E1-07 Lossy numeric casts in scheduling and rate limiting (med)

```
rust/crates/baukit-jobs/src/recurring.rs:95   ((self.seconds as f64 * jitter.fraction).floor() as u64).min(self.seconds as u64 - 1);
rust/crates/baukit-jobs/src/recurring.rs:97   let offset = chrono::Duration::try_seconds(offset as i64)
```

Also `rust/crates/baukit-ratelimit/src/memory.rs:80,95`, `rust/crates/baukit-auth/src/api_token.rs:596`, `rust/crates/baukit-suite/src/domain/mod.rs:495`. Line 95 also does `self.seconds as u64 - 1` which underflows when `seconds` is 0 (guard elsewhere, not local). Prevalence: 15 cast warnings across cast_possible_truncation 5, cast_precision_loss 4, cast_sign_loss 3, cast_possible_wrap 3 (totals above, overlapping lines). `grep -rnE --include="*.rs" " as (u8|u16|u32|i32|i64|u64|usize|f64|f32)\b" rust/crates/*/src | grep -v tests | wc -l` gives 38 raw `as` casts.

Prevention: `clippy::cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap`, `cast_precision_loss`, plus `clippy::as_conversions` if you want zero `as`. Fix cost: 15 (use `try_from`, `u64::from`).

### E1-08 Public fallible API without `# Errors` docs (med)

```
rust/crates/baukit-events/src/lib.rs:104   pub fn validate_event_envelope(
rust/crates/baukit-core/src/limits.rs:157  pub fn compact_json_utf8_bytes(value: &impl Serialize) -> Result<usize, serde_json::Error> {
```

baukit is a library consumed by 12 products, so this is its API contract. Prevalence: 249 in rust/, 6 in cli/ (totals above). Largest single item in the strict set.

Prevention: `clippy::missing_errors_doc`. Fix cost: 249. Mechanical but large; enable per crate starting with the most-consumed (core, http, config, auth).

### E1-09 `#[allow]` without reason, and `#[allow]` instead of `#[expect]` (low)

```
rust/crates/baukit-test/src/erasure.rs:26   #[allow(async_fn_in_trait)]
rust/crates/baukit-http/src/tests.rs:30     #[allow(dead_code)]
```

Prevalence: 11 `#[allow]` (6 dead_code, 5 async_fn_in_trait), 0 `#[expect]`. Command: `grep -rnE --include="*.rs" "#!?\[allow\(" rust/crates cli/src | wc -l` (run under bash; zsh globs `--include=*.rs`). Stale `allow`s stay forever, `expect` warns when the suppression is no longer needed.

Prevention: `clippy::allow_attributes` (forces `#[expect]`) and `clippy::allow_attributes_without_reason`. Fix cost: 11 (5 flagged in lib code by the latter, the rest in test targets).

### E1-10 Builder methods and pure accessors miss `#[must_use]` (low)

```
rust/crates/baukit-events/src/lib.rs:66
rust/crates/baukit-telemetry/src/lib.rs:191   (return_self_not_must_use)
```

A dropped builder return (`config.with_x(..);`) silently does nothing. Prevalence: 22 `return_self_not_must_use`, 146 `must_use_candidate` in rust/; 4 in cli/.

Prevention: `clippy::return_self_not_must_use` (high value, 22). `must_use_candidate` is noisy; I would skip it.

### E1-11 Async trait impls and async fns that never await (low)

```
rust/crates/baukit-http/src/extract.rs:120    (unused_async_trait_impl)
rust/crates/baukit-http/src/locale.rs:180     (unused_async_trait_impl)
rust/crates/baukit-jobs/src/runner.rs:251     (unused_async)
```

Prevalence: 7 + 1. Prevention: `clippy::unused_async`, `clippy::unused_async_trait_impl` (pedantic). Fix cost: 8.

### E1-12 CLI flag structs with many bools (low)

```
cli/src/lib.rs:190   pub struct NewOptions {
cli/src/lib.rs:328   (struct_excessive_bools)
```

`NewOptions` encodes backend/web/mobile/mcp/... as independent bools, which allows nonsense combinations the generator then has to validate. Prevalence: 4 in cli/, 1 in rust/ (`rust/crates/baukit-openapi/src/error_responses.rs:185`). Prevention: `clippy::struct_excessive_bools`, `clippy::fn_params_excessive_bools`. Fix cost: 5, real refactor (enum or bitflag set).

### E1-13 Timing-based waits in tests (low)

```
rust/crates/baukit-data-subject/tests/support/server_fixture.rs:535   tokio::time::sleep(Duration::from_millis(10)).await;
rust/crates/baukit-erasure/tests/api_providers.rs:156                 tokio::time::sleep(Duration::from_millis(10)).await;
rust/crates/baukit-jobs/src/runner.rs:891     std::thread::sleep(Duration::from_millis(2));   // inside #[cfg(test)] mod at 487, holding a std Mutex
```

Prevalence: 30 `sleep(` calls in rust/crates, 20 in `tests/` dirs (`grep -rnE --include="*.rs" "(tokio::time::sleep|thread::sleep)\(" rust/crates | wc -l` and `... | grep -c "/tests/"`). Several are poll loops, some are deliberate hang simulations (`rust/crates/baukit-egress/tests/guarded_client.rs:132`). Poll loops with fixed 10 ms steps flake on a loaded runner.

Prevention: `clippy::disallowed_methods` in clippy.toml listing `std::thread::sleep` and `tokio::time::sleep` with a reason pointing to `tokio::time::pause` or a notify-based wait; real cases get `#[expect]`. Fix cost: 30 to triage.

### E1-14 SQL DDL built with `format!` (low)

```
rust/crates/baukit-test/src/postgres_database.rs:133   execute(&mut admin, format!("CREATE DATABASE \"{name}\"")).await?;
rust/crates/baukit-test/src/postgres_database.rs:310   format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
```

Only 2 hits, both test-support DDL where Postgres cannot bind identifiers. `"` inside `name` is not escaped. Low risk since names are generated, but this code ships in a published crate. Command: `grep -rnE --include="*.rs" "format!\(\s*\"(SELECT|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP)" rust/crates cli/src templates`.

Prevention: no lint. Custom check needed, "fail on `format!` whose literal starts with an SQL keyword unless the file has a quote_ident helper call on the same line". Cheaper: add a `quote_identifier` helper and leave it.

### E1-15 CLI writes to stdout with `println!` from library-ish code paths (low)

```
cli/src/main.rs:128   println!("generated {}", destination.display());
cli/src/main.rs:101   (print_stderr)
```

All 7 hits are in `main.rs`, which is the right place. rust/ has 0. Prevalence: 6 print_stdout + 1 print_stderr. Prevention: `clippy::print_stdout`, `clippy::print_stderr` in `rust/` only (library crates must log through tracing). In cli/ keep them allowed in `main.rs`. Fix cost: 0 in rust/.

Not findings (checked, clean): Rust in templates is small (34 files, largest `templates/backend/backend/crates/__app__-domain/src/limits.rs` 526 lines), its 20 `expect/unwrap/panic` hits are in test modules (e.g. `templates/mcp-remote/backend/crates/__app__-mcp/src/lib.rs:116`), and it has 0 `#[allow]`. `std::sync::Mutex` in async crates (`rust/crates/baukit-ratelimit/src/memory.rs:18`) is not held across `.await`; `clippy::await_holding_lock` is already on and green.

## Recommended lint set

Same block in `rust/Cargo.toml` (`[workspace.lints.*]`), `cli/Cargo.toml` (`[lints.*]`) and `templates/backend/backend/Cargo.toml.jinja`. I did not measure the generated fixture; the template set should land after one fixture run.

```toml
[workspace.lints.rust]
unsafe_code = "deny"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
dbg_macro = "warn"
todo = "warn"
unwrap_used = "warn"
# new
expect_used = "warn"                    # rust 42 (12 build.rs, 15 baukit-test, 15 other) / cli 4
indexing_slicing = "warn"               # rust 26 / cli 16
cognitive_complexity = "warn"           # rust 3 / cli 2
too_many_lines = "warn"                 # rust 4 / cli 5 at 100; 8 / 11 at 80
cast_possible_truncation = "warn"       # rust 5
cast_sign_loss = "warn"                 # rust 3
cast_possible_wrap = "warn"             # rust 3
cast_precision_loss = "warn"            # rust 4
missing_panics_doc = "warn"             # rust 7
return_self_not_must_use = "warn"       # rust 22
allow_attributes = "warn"               # 11 #[allow] to convert
allow_attributes_without_reason = "warn" # rust 5 (lib) + test targets
unused_async = "warn"                   # rust 1
unused_async_trait_impl = "warn"        # rust 7
struct_excessive_bools = "warn"         # rust 1 / cli 4
needless_pass_by_value = "warn"         # rust 5
manual_let_else = "warn"                # rust 4
unimplemented = "warn"                  # 0
print_stdout = "warn"                   # rust 0 (cli: allow in main.rs)
print_stderr = "warn"                   # rust 0 / cli 1
disallowed_methods = "warn"             # see clippy.toml, 30 sleeps to triage
# phase 2, per crate
missing_errors_doc = "warn"             # rust 249 / cli 6
```

New `rust/clippy.toml` (copied to `cli/` and the template, kept in sync by the E1-06 check):

```toml
allow-expect-in-tests = true
allow-unwrap-in-tests = true
allow-indexing-slicing-in-tests = true
allow-panic-in-tests = true
too-many-lines-threshold = 80
disallowed-methods = [
  { path = "std::thread::sleep", reason = "use tokio::time or a notify-based wait" },
  { path = "tokio::time::sleep", reason = "tests: use tokio::time::pause/advance; runtime: use the backoff helper" },
]
```

Fix cost before turning on with `-D warnings`: about 130 violations for the core set (excluding missing_errors_doc), 255 more for missing_errors_doc. I skipped `pedantic` as a group: 146 `must_use_candidate` and 65 `doc_markdown` are noise relative to their value.

deny: `rust/deny.toml` `[bans]` add `allow-wildcard-paths = true`; CI runs `cargo deny ... check` (all), and a `cli/deny.toml` plus CI step.

Custom checks needed (no linter exists): lint-table drift across the three manifests (E1-06), file size cap with shrinking allowlist (E1-02), SQL `format!` guard (E1-14, optional).
