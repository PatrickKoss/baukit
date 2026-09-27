---
'@baukit/data-contracts-expo-sqlite': minor
---

Add the `@baukit/data-contracts-expo-sqlite/testing` entry point for Node unit tests on the built-in `node:sqlite` (Node 24 or later, no new dependency). `NodeSqliteDatabase` implements the Expo statement methods this adapter calls, on a file path it owns or one you pass in. Exclusive transactions open a second connection to the same file, as on the device, so an overlapping root write fails with `database is locked`, and foreign keys stay off on every connection. `createSqliteMigrationConformanceTests(adapter)` returns framework-neutral cases that check a product migration runner for fresh installs, upgrades, restarts, complete rollback of a failed step, and refusal of a database migrated by a newer build.

Export `ExpoSqliteDatabase`, the part of an Expo `SQLiteDatabase` the adapter calls. The `ExpoSqliteStore` constructor now accepts that type instead of `SQLiteDatabase`, so an Expo database still fits.

No breaking changes.
