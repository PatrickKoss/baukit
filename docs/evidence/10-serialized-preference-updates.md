# Evidence for serialized preference updates

## Source product files

- `redemut/packages/preferences/src/index.ts`: `SerializedRedemutPreferenceController`
- `redemut/packages/preferences/test/preferences.test.ts`: rapid-write, failed-write, effect, and
  identity-switch tests
- `redemut/docs/baukit-playback-audit.md`: package-boundary analysis

## Observed repeated glue

Redemut wraps `@baukit/preferences-core` to prevent concurrent optimistic writes from reading the
same prior state and settling out of order. The wrapper owns a promise queue, committed-value
publication, pending state, failure recovery, and identity generations.

## Baukit owner

`@baukit/preferences-core` owns update ordering, visible controller state, and invalidation when an
identity changes or a controller stops publishing.

## Public types and errors

- `PreferenceUpdateMode`: `"optimistic" | "serialized"`
- `SerializedPreferenceController<TValues>`
- `SerializedPreferenceControllerState<TValues>` with `pendingCount`
- `createPreferenceController({ updateMode: "serialized" })`

The mode adds no error class. It preserves store and side-effect errors, including the existing
aggregate error when persistence and preview rollback both fail. Unknown keys reject with
`TypeError` before entering the queue.

## Product-owned inputs

Products still provide preference definitions, normalizers, scopes, the store, side effects, and
the visible-state callback. Baukit does not own preference names, schemas, UI, or sync policy.

## Required cases

- Concurrency: rapid patches run one at a time against the latest committed result.
- Failure: a failed patch rolls back previews, does not enter the next patch, and does not stop the
  queue.
- Privacy: errors contain no preference values unless a product store puts them in its own error.
- Cleanup: identity changes block stale publication and old queued writes; `stop()` blocks all later
  callbacks while queued and in-flight work settles.

## Supported runtimes

The controller uses promises and injected storage only. It supports the package's Node 24+ ESM
runtime and browser or React Native bundles that consume the ESM build.

## Product adoption pull request

No adoption pull request exists yet. After release, Redemut can replace its controller construction with
`createPreferenceController({ updateMode: "serialized" })`, expose the returned `pendingCount`, and
delete `SerializedRedemutPreferenceController` from `packages/preferences/src/index.ts`. Its tests
can stop testing the local queue and retain product definition and repository-mapping coverage.
