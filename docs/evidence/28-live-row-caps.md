# Item 28 evidence: PostgreSQL live-row caps

## Source product files

- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/row_caps.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/practice.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/programs.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/soundscape.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/sync.rs`
- `/home/patrick/projects/eigenruhe/backend/migrations/20260823000002_practice.sql`
- `/home/patrick/projects/eigenruhe/backend/migrations/20260823000004_programs.sql`
- `/home/patrick/projects/eigenruhe/backend/migrations/20260823000006_adaptation_soundscapes.sql`
- `/home/patrick/projects/eigenruhe/backend/tests/postgres_integration.rs`

## Observed failure or repeated glue

`row_caps.rs` builds product SQL for per-owner, per-program, and per-UTC-day counts. It skips
tombstones and existing live rows, then runs `COUNT(*)` before the write in the same default-isolation
transaction. It locks no stable scope row. A synchronized PostgreSQL 18 Docker control accepted both
last-slot creates in 16 of 16 runs and left three live rows under a cap of two.

The product integration test checks boundaries, updates, and tombstone release sequentially. It does
not race two database connections.

## Baukit owner

`docs/platform/live-row-caps.md` owns the PostgreSQL recipe. `baukit-test` owns the product-neutral
race, count, update, and soft-delete conformance sequence.

## Public types and errors

The additive API exports `PostgresLiveRowCapAdapter`, `LiveRowCapConformanceCases`,
`LiveRowCapConformanceError`, `check_postgres_live_row_cap_conformance`, and
`assert_postgres_live_row_cap_conformance`. The conformance error contains fixed violation text and
counts. It never formats the adapter error, row, owner, or parent.

## Product-owned inputs

Products keep SQL, schema identifiers, scope columns, cap values, day boundaries, stable limit codes,
database-error mapping, retry limits, counter repair, and the decision between row locks,
serializable transactions, counters, and fixed slots.

## Concurrency, failure, privacy, and cleanup cases

- Concurrency: the helper fills `limit - 1` rows and polls two distinct creates concurrently. It
  requires one success, one stable-code rejection, and an exact live count at capacity.
- Failure: a count failure, early rejection, wrong stable code, double acceptance, double rejection,
  update failure, soft-delete failure, or replacement failure produces a typed conformance error.
- Privacy: adapter errors and row values do not enter conformance output. Product stable codes must
  contain no owner or payload data.
- Cleanup: the adapter uses a clean scope. Its transaction must roll back inserts, reservations, and
  counter changes together. Soft deletion releases capacity once, even when retried.

The Docker suite repeated each safe method 16 times. Row locking returned 16 post-lock capacity
rejections. Serializable transactions produced 16 SQLSTATE `40001` retries, each followed by a
capacity rejection. Counters produced 16 conditional reservation misses. Fixed slots produced 16
SQLSTATE `23505` conflicts naming the live-slot index. Each method accepted one raced create, allowed
an update at capacity, released a slot on soft deletion, and accepted a replacement in all runs.

## Supported runtimes

The conformance API supports Rust 1.95 or newer on Tokio. The adapter is database-library neutral but
targets PostgreSQL semantics. The crate's own ignored integration test uses SQLx, PostgreSQL 18
Alpine, and Docker.

## Product adoption change

Eigenruhe should replace the unprotected count operations in `row_caps.rs` with one documented
enforcement method and implement `PostgresLiveRowCapAdapter` around its repository. It can then delete
`assert_practice_cap` and the duplicated update and tombstone-release blocks in
`backend/tests/postgres_integration.rs`. The production file remains because entity SQL, scope, and
limit-code mapping are product work.

## Implementation report

### 1. Summary

Added a PostgreSQL recipe and an additive `baukit-test` conformance API. The helper binds to a
product adapter, fills all but one slot, polls two creates concurrently, checks for one stable-code
rejection, updates at capacity, soft-deletes a row, and creates a replacement. Its errors omit
adapter details and identifiers.

The Docker test compares a stable scope-row lock, serializable transactions with bounded retries, a
maintained counter, and fixed slots backed by a partial unique index. It also includes an unsafe
count-then-insert control. Baukit owns only the test sequence. Products retain schema, SQL, scopes,
caps, and error mapping. `postgres.rs` needed no extension. There was no plan deviation.

### 2. Files added or changed

- `docs/evidence/28-live-row-caps.md`
- `docs/platform/live-row-caps.md`
- `rust/crates/baukit-test/CHANGELOG.md`
- `rust/crates/baukit-test/README.md`
- `rust/crates/baukit-test/src/lib.rs`
- `rust/crates/baukit-test/src/live_row_cap.rs`

### 3. Verification

- `cargo fmt --manifest-path rust/Cargo.toml --all && cargo test --manifest-path rust/Cargo.toml -p
  baukit-test --no-run`: the first development run failed with two compile errors and one warning.
  The errors and warning were fixed.
- `cargo fmt --manifest-path rust/Cargo.toml --all && cargo test --manifest-path rust/Cargo.toml -p
  baukit-test --no-run && cargo clippy --manifest-path rust/Cargo.toml -p baukit-test --all-targets
  -- -D warnings`: one development run failed with one type error. The next run passed compilation
  and clippy with zero warnings.
- `cargo test --manifest-path rust/Cargo.toml -p baukit-test
  live_row_cap::tests::compares_live_row_cap_methods_on_postgres -- --ignored --nocapture`: the first
  development run failed because SQLx rejected a multi-command prepared statement. It was changed to
  raw SQL execution.
- `cargo fmt --manifest-path rust/Cargo.toml --all && cargo test --manifest-path rust/Cargo.toml -p
  baukit-test live_row_cap::tests::compares_live_row_cap_methods_on_postgres -- --ignored
  --nocapture`: the first run exposed insufficient synchronization in the unsafe control. After a
  test barrier was added, the rerun passed 1 test.
- `cargo fmt --manifest-path rust/Cargo.toml --all --check && cargo test --manifest-path
  rust/Cargo.toml -p baukit-test`: passed 57 tests with 7 ignored and 5 doctests.
- `git diff --check -- <owned paths>` and the prose-pattern scan: passed with no whitespace errors or
  matches in the new recipe, evidence, or Rust module.
- `cargo test --manifest-path rust/Cargo.toml -p baukit-test --no-default-features --no-run`: passed;
  the SQLx-backed internal test stays behind `sqlx-postgres`.
- `cargo test --manifest-path rust/Cargo.toml -p baukit-test -- --include-ignored`: passed on all four
  runs. The final run passed 64 tests and 5 doctests with no ignored tests left.
- `cargo fmt --manifest-path rust/Cargo.toml --all --check`: passed on every final run.
- `cargo clippy --manifest-path rust/Cargo.toml -p baukit-test --all-targets -- -D warnings`: passed
  on every final run with zero warnings.
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings`: passed for
  the 16-crate workspace with zero warnings.
- `cargo check --manifest-path rust/Cargo.toml --workspace --all-targets`: passed for all 16 crates.
- `cargo +1.95 check --manifest-path rust/Cargo.toml --workspace --all-targets`: passed for all 16
  crates.
- `cargo test --manifest-path rust/Cargo.toml --workspace -- --include-ignored`: passed on all four
  runs. The final run passed 381 unit and integration tests plus 55 doctests, with no failures or
  ignored tests.
- `cargo fmt --manifest-path rust/Cargo.toml --all --check && cargo clippy --manifest-path
  rust/Cargo.toml -p baukit-test --all-targets -- -D warnings && cargo test --manifest-path
  rust/Cargo.toml -p baukit-test live_row_cap::tests::compares_live_row_cap_methods_on_postgres --
  --ignored`: passed formatting, clippy with zero warnings, and 1 Docker test after the bounded-retry
  edit.
- `make ci`: failed in the concurrent CLI generator suite after Rust and TypeScript checks passed.
  The CLI result was 30 passed and 7 failed.

Docker-gated suites ran. The package run covered the new PostgreSQL cap race, PostgreSQL inbox,
PostgreSQL startup, migrations and foreign-key audit, Redis, and Redis Sentinel. The workspace run
also covered 12 `baukit-jobs` PostgreSQL tests, 10 `baukit-ratelimit` Redis tests, and 9 `baukit-sync`
PostgreSQL tests.

### 4. Failures observed in other agents' areas

`make ci` failed seven CLI generator tests: `doctor_requires_generated_environment_and_strict_markdown_scripts`,
`generated_markdown_link_check_fails_for_a_committed_missing_target`,
`generated_migration_guard_ports_failure_cases`,
`mcp_generation_matches_golden_tree_and_records_personal_token_auth`,
`oidc_generation_is_deterministic_and_records_the_optional_capability`,
`quality_flag_generates_the_strict_profile`, and
`strict_generation_is_capability_driven_and_matches_golden_tree`. Their failures were golden-tree or
generated-script differences in paths owned by another agent.

### 5. Leftovers and open questions for the orchestrator

The orchestrator must resolve the concurrent CLI snapshot failures and rerun `make ci`. Product
adoption is still required for plan completion. No dependency, lockfile, or version changed, so cargo
deny and version-coherence checks did not apply.

### 6. Product adoption note

Eigenruhe can delete `assert_practice_cap` and the local update and tombstone-release assertion blocks
in `backend/tests/postgres_integration.rs` after it adopts the release and fixes production locking.
