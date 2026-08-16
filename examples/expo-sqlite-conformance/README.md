# Expo SQLite device conformance

This Expo SDK 57 app executes the cases from the shared
`@baukit/data-contracts/vitest` suites against the real
`@baukit/data-contracts-expo-sqlite` adapter. A tiny native runner mirrors the
Vitest cases because Vitest itself requires Node and cannot run inside React
Native. It also proves database creation and reopening, namespace isolation,
malformed-row redaction, rollback, and schema-metadata upgrades.

On Linux with Java 21 and KVM available:

```sh
make expo-sqlite-conformance
```

The target installs the pinned Android API 36 command-line tools and emulator
under `$HOME/Android/Sdk`, boots an ephemeral headless emulator, builds and installs
the debug app, starts Metro, and fails unless logcat contains
`BAUKIT_SQLITE_CONFORMANCE_PASS`. Diagnostics are retained in `artifacts/`.

iOS uses the same JavaScript runner, but compiling and executing it requires a
macOS runner with Xcode and an iOS Simulator. Linux results are never recorded
as an iOS pass; iOS is a scheduled/manual platform gate.
