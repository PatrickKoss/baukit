# ADR 0026: Crates ship reference SQL that products copy and apply as expand and contract steps, never on API startup

## Status

Accepted, 2026-11-27.

## Context

Several crates own PostgreSQL tables: jobs, auth, sync, push, integrations,
erasure, data subject and suite. Products run one migration sequence per
database, under sqlx, which checksums each applied file. A crate that ran its
own migrations would fight the product's sequence and its checksums.

Products migrate databases that serve traffic while the previous release's
binaries still run. A migration that takes a long lock or drops a column the old
binary reads breaks the rollout, and an edited shipped file breaks every product
that already applied it.

## Decision

Each crate keeps its SQL under `rust/crates/<crate>/migrations/` as reference
files. Products copy them into their own `migrations/` directory, in the order
`rust/migrations.toml` lists. Crates apply in list order, each crate's files in
file-name order. The `after` key names the crates whose tables a crate's SQL
touches. `baukit-erasure`, `baukit-data-subject` and `baukit-suite` come after
`baukit-jobs`.

No crate runs migrations itself. `baukit-test` applies them only to test
containers. The generated backend has a separate `migrate` binary,
`templates/backend/backend/crates/__app__-bin/src/bin/migrate.rs`. The Helm
chart runs it as a `pre-install,pre-upgrade` hook job when `migration.enabled`
is true, which is the default in `deploy/chart/baukit-app/values.yaml`. The API
binary does not migrate on startup.

New files follow the rules in
[authoring SQL migrations](../migrations/authoring-sql-migrations.md): never
edit a shipped file, set a lock timeout, create indexes concurrently, add
constraints `NOT VALID` and then validate, expand before contract across
releases, keep data changes out of schema files, and make every statement
re-runnable. The `[shipped]` list in `rust/migrations.toml` holds files that
predate the rules. It only shrinks.

## Enforced by

The `migrations` gate step runs three commands.

- `python3 scripts/check-migrations.py lint`. Before linting, it rejects a crate
  listed twice, a crate with migrations that `rust/migrations.toml` does not
  list, a listed crate without migrations, a crate listed before one it names in
  `after`, and a `[shipped]` entry whose file is missing. It then runs squawk for
  PostgreSQL 18 over every file not in `[shipped]`, with
  `--assume-in-transaction` unless the file starts with `-- no-transaction`.
- `python3 scripts/check-migrations.py double-apply`. It applies every file in
  order to a fresh `postgres_image` container and fails when one errors. It then
  reruns each file not in `[shipped]` and fails when a rerun errors or the schema
  dump changes.
- `sh scripts/migrations-old-binary.sh` runs the previous release's store tests
  against HEAD's schema. `[old_binary] tests` is empty today, so this check
  passes without running anything. Expand and contract has no machine check
  until that list has entries.

Nothing checks that an edited shipped file keeps its checksum. Comparing each
`[shipped]` file's hash against the previous release tag would.

## Consequences

Products own their migration sequence and can review every statement before it
runs. They copy files by hand on each upgrade and must keep the listed order.
Guides under `docs/migrations/` name the new files for a breaking change.
A breaking schema change takes three releases.

## History

- 2026-08-08, `32eb9165`: the first templates shipped the separate `migrate`
  binary and the chart's migration hook job.
- 2026-11-25, `274fc8d9`: `rust/migrations.toml`, the squawk lint, the
  double-apply check and the old-binary script were added. Before that, order
  and safety rules lived only in crate READMEs.
