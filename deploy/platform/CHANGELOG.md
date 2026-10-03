# Changelog

## [Unreleased]

- Pin the CNPG operator default, primary and restore-test clusters to PostgreSQL
  `18.6-system-trixie`.
- Move PostHog's PostgreSQL dependency to digest-pinned `18.6-alpine`. Mount
  `/var/lib/postgresql` and set `PGDATA` to `/var/lib/postgresql/18/docker`.
  Recreate local database PVCs for the major-version change.
