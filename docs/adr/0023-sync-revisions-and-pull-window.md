# ADR 0023: Sync orders rows by per-owner revisions in a bounded pull window, and HLC orders writes only

## Status
Accepted, 2026-11-27.

## Context
A client pulls everything above the cursor it last saw. Wall-clock timestamps cannot be that cursor: two rows in one millisecond tie, and clock steps move rows behind a cursor.

A revision alone was not enough either. One push stamps one revision on several rows, so a cursor that ends inside a revision skips the rest. Under READ COMMITTED a writer can commit between two table reads, and per-table `LIMIT`s let the highest last row skip rows lower in a fuller table.

## Decision
`baukit-sync` keeps one counter per owner in `sync_revisions`. `next_revision` allocates a revision inside the writing transaction. A pull calls `pull::open_pull_window` first, which fixes an upper bound `hi` and returns `resync_required` for a cursor above the owner's counter. Each table reads `WHERE owner_id = $1 AND revision <= $2 AND (revision, id) > ($3, $4) ORDER BY revision, id LIMIT $5`, and `merge_pull_page` cuts the merged rows at the lowest full table and returns a `(revision, id)` cursor. The limit is a `NonZeroUsize`.

`hlc::HybridLogicalClock` orders concurrent writes for merge decisions. It never drives the pull cursor. `observe` rejects remote timestamps more than 24 hours ahead. See the [crate README](../../rust/crates/baukit-sync/README.md).

## Enforced by
- Verus V-01, `replay` in `rust/verus/src/hlc.rs`: outputs strictly increase and drift stays bounded. Kani equivalence harnesses `now_step_equivalent`, `observe_step_equivalent` and `codec_equivalent` in `rust/verus/equivalence/src/hlc.rs` tie the twin to production.
- Verus V-03, `page_progress_and_no_skip` in `rust/verus/src/pull.rs`: each page advances the cursor or ends, and no row inside the window is skipped. Gate steps `verus` and `kani`.
- `per_table_limits_do_not_skip_rows_of_the_shorter_table`, `window_bounds_a_read_committed_pull_across_tables` and `unsafe_cursors_fail_the_conformance_helper` in `rust/crates/baukit-sync/tests/pull_window.rs`. Gate step `rust-tests`.
- `baukit_test::check_pull_conformance` for product endpoints.

## Consequences
Products get gap-free pulls across tables. They must return both cursor parts and keep a returning owner's counter instead of deleting it.

## History
- 2026-11-25, `7d9fd1df`: the revision-only cursor and `guard_pull_cursor` replaced by `open_pull_window` and the `(revision, id)` tuple. A bare revision maps to `PullCursor::after_revision`.
- 2026-11-25, `0e3e7839`: `observe` gained the 24 hour drift bound. Before, one far-future timestamp pushed every later one forward.
