# Authoring SQL migrations

baukit ships reference migrations in `rust/crates/*/migrations/`. Products copy them into their own
ordered migrations and run them against databases that are serving traffic, while the previous
release's binaries are still running. Every rule below follows from that: a migration must be safe to
apply on a live database, and the schema it leaves must still work for the old binary.

The `migrations` gate step (`make migrations-check`) enforces what a machine can check. Review covers
the rest.

## Rules

1. **Never edit a shipped file.** sqlx stores a checksum per applied migration, so a changed file
   breaks every product that already ran it. Fix a shipped migration with a new file. Files shipped
   before these rules are listed under `[shipped]` in `rust/migrations.toml`; that list only shrinks.

2. **Set a lock timeout first.** Start every migration with `SET lock_timeout = '5s';` (and a
   `statement_timeout` for anything that scans). A DDL statement waiting behind a long transaction
   otherwise queues every later query on the table behind it.

3. **Create indexes concurrently.** Use `CREATE INDEX CONCURRENTLY IF NOT EXISTS`. It cannot run in a
   transaction, so put it in its own file whose first line is `-- no-transaction`, the sqlx marker.
   sqlx sends such a file as one query, and PostgreSQL runs a query with several statements as one
   transaction block, so the file holds only the `CREATE INDEX` and no `SET`. Suppress squawk's
   timeout rules there with a reason. An index on a table created in the same file is exempt.

4. **Add constraints `NOT VALID`, then `VALIDATE`.** `ALTER TABLE ... ADD CONSTRAINT ... NOT VALID`
   takes a short lock and checks only new rows. A later statement or file runs
   `ALTER TABLE ... VALIDATE CONSTRAINT ...`, which scans without blocking writes. This applies to
   CHECK and foreign keys, including the owner foreign keys products add.

5. **Expand, then contract.** Never rename a column, change its type, drop it, or add `NOT NULL`
   without a default in one step. Release N adds the new shape (nullable column, new table, dual
   writes). Release N+1 backfills in batches and switches reads. Release N+2 drops the old shape. The
   old binary must work against the schema of every step, which the old-binary check tests.

6. **Never delete or rewrite rows in a schema migration.** Data changes go in a job that runs in
   batches and can resume.

7. **Make every statement re-runnable.** Use `IF NOT EXISTS`, `IF EXISTS`, `CREATE OR REPLACE` and
   `DROP ... IF EXISTS` before re-creating triggers. The double-apply check runs every new file twice.

8. **Respect cross-crate order.** `rust/migrations.toml` lists the crates in the order a product
   applies them, and each crate names the crates whose tables it touches under `after`. A migration
   that adds a dependency on another crate's table updates that entry. The checker fails when the list
   contradicts `after` or misses a crate.

## What CI checks

| Check | Command | Catches |
|---|---|---|
| Order | `scripts/check-migrations.py order` | a crate missing from the list, or listed before a crate it needs |
| Lint | `scripts/check-migrations.py lint` | squawk findings in any file not in `[shipped]`: missing lock timeout, non-concurrent index, validated constraint, unsafe column change |
| Double apply | `scripts/check-migrations.py double-apply` | a new file that fails on its second run, or a second run that changes the schema |
| Old binary | `scripts/migrations-old-binary.sh` | the previous release's store tests failing against HEAD's schema |

The old-binary check runs the store tests listed in `[old_binary] tests` of
`rust/migrations.toml`. The list is empty today and the step passes. A test can only be
listed once a release ships whose baukit-test reads `BAUKIT_TEST_DATABASE_URL`, so the
list fills after the next release.

A squawk finding that is safe in context can be suppressed in the file with
`-- squawk-ignore <rule>` and a comment that says why.
