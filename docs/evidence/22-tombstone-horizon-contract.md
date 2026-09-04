# Tombstone horizon and full-resync evidence

## Source product files

- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-postgres/src/retention.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-api/src/error.rs`
- `/home/patrick/projects/tiefgang/mobile/src/sync/engine.ts`
- `/home/patrick/projects/tiefgang/mobile/src/db/dexie/sync-persistence.web.ts`
- `/home/patrick/projects/tiefgang/mobile/src/db/sqlite/sync-persistence.ts`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/retention.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-services/src/sync.rs`
- `/home/patrick/projects/eigenruhe/mobile/src/sync/engine.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/sync/retention.ts`

## Observed failure or repeated glue

Both products purge tombstones, keep a per-owner horizon, reject stale nonzero cursors, and rebuild
local data from cursor zero. Tiefgang preserves pending and rejected entities and resets its cursor
in the same transaction. Eigenruhe clears rows and stores cursor zero in separate transactions, and
its `resync_required` response does not yet include the horizon.

## Baukit owner

The platform contract owns the lifecycle rules. `@baukit/sync-client/conformance` owns the optional,
storage-neutral reset cases. Products keep the server and client implementations.

## Public types and errors

`SyncConformanceStaleCursor<TCursor>` normalizes `resync_required` and its horizon.
`SyncConformanceFullResyncCallbacks` supplies cursor zero, stale-error decoding, the safety hook,
the atomic reset, reset observation, and fake-server purge faults. The wire contract uses
`error.code = "resync_required"` and `error.details.horizon_revision`.

## Product-owned inputs

Products own owner keys, cursor representation, retention time, tables, foreign-key deletion order,
SQL, safe-reset policy, entity payloads, conflict handling, and user copy.

## Cases

- Concurrency: a reset keeps pending edits and prevents the rebuilt snapshot from replacing them.
- Failure: reset rollback preserves rows and cursor; unsafe reset defers; a second stale response
  stops after one reset.
- Privacy: the stale response contains only a code and horizon. Fixtures use opaque entity data.
- Cleanup: parent, child, and pull-only server rows are cleared; pending and rejected repair state
  remains. Scenario disposal stays with the existing adapter callback.

## Supported runtimes

ES2022 runtimes supported by `@baukit/sync-client`, including Node 24 or newer, browsers, and
compatible React Native JavaScript engines. Storage adapters must provide an atomic write
transaction.

## Product adoption change

Tiefgang can replace its local horizon/reset cases in `mobile/src/sync/engine.test.ts` and
`mobile/src/db/repository-contract.ts` with the shared conformance callbacks. Eigenruhe can replace
its reset case in `mobile/src/sync/engine.test.ts` after it adds the horizon to the error envelope
and combines row clearing with cursor zero in one transaction. The product repositories are
read-only in this batch, so neither adoption change has run yet.

## Implementation report

### 1. Summary

The offline contract now defines per-owner purge horizons, the cursor-zero rebuild request, the
stable stale-cursor envelope, reset preservation and atomicity, reset deferral, and the repeated
stale stop condition. The conformance adapter has one optional `fullResync` callback group. When
present, the harness adds five cases without changing existing adapters. Baukit does not add server
helpers, table registration, deletion SQL, or a sync engine.

The plan's two-product adoption condition is not complete in this read-only product pass.
Eigenruhe must make its reset atomic and return the horizon before it can enable the cases.

### 2. Files added or changed

- `agent-skills/skills/baukit-offline-sync/SKILL.md`
- `docs/evidence/22-tombstone-horizon-contract.md`
- `docs/platform/offline-readiness-contract.md`
- `typescript/.changeset/tombstone-horizon-conformance.md`
- `typescript/packages/sync-client/README.md`
- `typescript/packages/sync-client/src/conformance.test.ts`
- `typescript/packages/sync-client/src/conformance.ts`
- `typescript/packages/sync-client/type-tests/conformance.ts`

### 3. Verification

- `corepack pnpm --dir typescript install --frozen-lockfile`: passed, 19 workspace projects,
  lockfile current.
- `corepack pnpm --dir typescript --filter @baukit/sync-client run build`: passed.
- `corepack pnpm --dir typescript --filter @baukit/sync-client run lint`: failed on one unchanged
  line in `src/expo.ts`; the owned conformance files have no lint errors.
- `corepack pnpm --dir typescript --filter @baukit/sync-client run test`: passed, 8 files and 145
  tests. Earlier development runs failed at 143 of 144, then 144 of 145, before the fixture and
  compatibility-count corrections; the next runs passed.
- `corepack pnpm --dir typescript --filter @baukit/sync-client run format:check`: passed.
- `corepack pnpm --dir typescript run check`: failed, 71 of 72 tasks passed. Only
  `@baukit/sync-client#lint` failed on unchanged `src/expo.ts:23`.
- `make ts-browser-test`: passed, 2 files and 26 tests across Chromium and WebKit.
- `make ci`: failed at `ts-lint` on unchanged `src/expo.ts:23`. Before that failure, install,
  formatting for 18 packages, and Rust formatting passed. Later CI stages did not run.
- `corepack pnpm --dir typescript exec eslint packages/sync-client/src/conformance.ts packages/sync-client/src/conformance.test.ts --max-warnings=0`:
  passed.
- `corepack pnpm --dir typescript exec prettier --check packages/sync-client/README.md ../docs/platform/offline-readiness-contract.md ../docs/evidence/22-tombstone-horizon-contract.md ../agent-skills/skills/baukit-offline-sync/SKILL.md .changeset/tombstone-horizon-conformance.md`:
  passed after formatting the README. The first check found that README formatting difference.
- `git diff --check -- docs/platform/offline-readiness-contract.md typescript/packages/sync-client/src/conformance.ts typescript/packages/sync-client/src/conformance.test.ts typescript/packages/sync-client/type-tests/conformance.ts typescript/packages/sync-client/README.md agent-skills/skills/baukit-offline-sync/SKILL.md`:
  passed.
- Docker-gated suites: none apply because this item changes no Rust crate or Docker-backed adapter.

### 4. Failures in other areas

`typescript/packages/sync-client/src/expo.ts:23` fails
`@typescript-eslint/no-unnecessary-type-assertion`. The file is unchanged and outside this item's
ownership. It blocks the package lint script, workspace check, and `make ci`.

### 5. Leftovers and open questions

- Tiefgang and Eigenruhe still need product adoption changes against a released package.
- Eigenruhe must include `horizon_revision` in `resync_required` and combine row clearing with
  cursor zero in one transaction.
- The orchestrator must assign the unrelated Expo lint fix to its owner before the full gate passes.

### 6. Product adoption note

No complete product implementation file becomes deletable because this item adds contract and
conformance only. Tiefgang can remove its horizon/reset cases from
`mobile/src/sync/engine.test.ts` and `mobile/src/db/repository-contract.ts`. Eigenruhe can remove the
matching case from `mobile/src/sync/engine.test.ts` after meeting the two contract gaps above.
