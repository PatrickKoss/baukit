# M-01 SyncPull

A paged sync pull for one owner against concurrent writers and tombstone purge. Settles F-06 and F-07.

## What maps to what

| Model | Code |
|---|---|
| `counter`, writers commit in revision order | `sync_revisions` row, `baukit-sync/src/revision.rs` (`UPDATE ... RETURNING` holds the row lock until commit) |
| `WriteSameRev` | one revision stamped on every row of a push (`baukit-sync/src/lib.rs` example, F-06) |
| `WriteCross` | one transaction writing two tables with two revisions (F-07) |
| `Purge` | `purge_tombstones` and `raise_horizons` (`baukit-sync/src/purge.rs`); it locks `sync_revisions` `FOR UPDATE`, so it waits for an open pull |
| `Begin` | `guard_pull_cursor` (`FOR KEY SHARE` plus `check_pull_cursor` in `horizon.rs`); resync on a cursor below the horizon |
| `ReadTable` | one `SELECT ... WHERE revision > $cursor ORDER BY revision LIMIT n` per table (`baukit-sync/README.md` pull example) |
| `Finish` | commit, then the client stores the largest revision it saw |

Writers are one interleaving of atomic commits: per-owner revisions serialize them, so two writers add
nothing a single nondeterministic writer does not.

Fixed design (`Design = "fixed"`): `Begin` reads the high-water mark `hi` first, every query filters
`revision <= hi` and uses a `(revision, id)` tuple cursor, and the client applies only the smallest key
across tables (a merged page), or jumps to `(hi, max id)` when nothing is left.

## Invariants

- `NoLostUpdate`: with no pull open, every row whose `(revision, id)` is at or below the cursor is in the
  replica at that version.
- `NoResurrection`: a purged row is never live in a replica whose cursor the guard accepts.
- `HorizonSafety`: no purge raises the horizon above the cursor of an open pull.

## Configs

| Config | Expect | Constants |
|---|---|---|
| `SyncPull.cfg` | pass | fixed, RC, paged, same-revision writes on |
| `SyncPull_current.cfg` | violation (F-06) | current, RC, paged, same-revision writes on |
| `SyncPull_crossTable_current.cfg` | violation (F-07) | current, RC, unpaged, same-revision writes off |

Rows 1 to 3 (row 3 in table 2), revisions up to 3.

## Counterexamples on today's code

F-06 (`SyncPull_current.cfg`, `NoLostUpdate`): one push stamps revision 1 on rows 1 and 2. The pull asks
table 1 for `revision > 0 LIMIT 1` and gets row 1. The client stores cursor 1. Row 2 is at revision 1, so
`revision > 1` never returns it.

F-07 (`SyncPull_crossTable_current.cfg`, `NoLostUpdate`): row 1 is written at revision 1. The pull reads
table 1 and gets row 1. A writer then commits row 1 at revision 2 and row 3 (table 2) at revision 3. Under
READ COMMITTED the pull's next statement reads table 2 and sees revision 3. The client stores cursor 3 and
never sees row 1 at revision 2.

With `Paged = TRUE` and same-revision writes off, TLC also finds a third loss: with one `LIMIT` per table and
a cursor set to the largest revision across tables, a full page in table 1 plus a higher revision in table 2
skips the rest of table 1. The fixed design's merged page avoids it.

## Paired test

P-11 (sync state machine on Postgres) runs the same actions against the real tables at RC and RR and checks
that the replica equals the reference after a full pull.
