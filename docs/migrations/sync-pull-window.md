# Pull with a window and a (revision, id) cursor

Three pull bugs lose rows silently:

- A push stamps one revision on several rows. A revision-only cursor that
  ends inside a revision skips the rest of it.
- Under READ COMMITTED a writer can commit between two table reads, so the
  client stores a cursor past a row it never saw.
- Each table query has its own `LIMIT`. Storing the highest last row across
  tables skips rows of a table that filled its limit lower down.

Replace `guard_pull_cursor` with `baukit_sync::pull::open_pull_window` as the
first call in the pull transaction. Read every table with

```sql
WHERE owner_id = $1 AND revision <= $2 AND (revision, id) > ($3, $4)
ORDER BY revision, id
LIMIT $5
```

binding `window.hi`, `window.after.revision` and `window.after.id`, and build
the response with `merge_pull_page`. Return the new cursor as both
`revision` and `id`. A client that still sends a bare revision maps to
`PullCursor::after_revision(n)`, which is safe.

Test the endpoint with `baukit_test::check_pull_conformance` (two tables,
per-table limit 1, writes during paging).

`open_pull_window` returns `resync_required` for a cursor above the owner's
counter. That covers a deleted and recreated owner only until new writes pass
the old cursor. If an owner id can return, keep its counter and raise its purge
horizon instead of deleting the `sync_revisions` row.
