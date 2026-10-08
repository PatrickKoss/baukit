# Expo SQLite device conformance

This Expo SDK 57 app executes the cases from the shared
`@baukit/data-contracts/vitest` suites against the real
`@baukit/data-contracts-expo-sqlite` adapter. A tiny native runner mirrors the
Vitest cases because Vitest itself requires Node and cannot run inside React
Native. It also proves database creation and reopening, namespace isolation,
malformed-row redaction, rollback, and schema-metadata upgrades.
Three cases issue root record, key/value, and schema-metadata calls against an
exclusive transaction in both arrival orders, including two stores that share
one database handle. Without the adapter's per-file operation queue they fail
with `database is locked` or read uncommitted state.
Five cases run `ExpoSqliteConnection` on the same handle: a root write issued
during another caller's transaction stays out of it, foreign keys and
`ON DELETE CASCADE` hold inside a transaction, a failed step rolls back its
schema changes, nested transactions run as savepoints without waiting on the
file queue, and the connection and an `ExpoSqliteStore` on one file wait for
each other in both arrival orders.
The native runner also opens distinct SHA-256-derived database files for an
offline E→F→E switch and proves record/outbox isolation, close-before-open,
memory reset, one-time legacy claiming, corrupt-registry blocking, terminal
session expiry, and server-subject mismatch behavior. Expo Crypto is injected
as the React Native SHA-256 implementation.

On Linux with Temurin 25.0.4.1+1 and KVM available:

```sh
make expo-sqlite-conformance
```

The target installs the pinned Android API 36 command-line tools and emulator
under `$HOME/Android/Sdk`, boots an ephemeral headless emulator, builds and installs
the debug app, starts Metro, and fails unless logcat contains
`BAUKIT_SQLITE_CONFORMANCE_PASS`. Diagnostics are retained in `artifacts/`.
Set `METRO_PORT` to run Metro on a host port other than 8081. The script maps the device's fixed
`localhost:8081` debug endpoint to that host port with `adb reverse`.

iOS uses the same JavaScript runner, but compiling and executing it requires a
macOS runner with Xcode and an iOS Simulator. Linux results are never recorded
as an iOS pass; iOS is a scheduled/manual platform gate.
