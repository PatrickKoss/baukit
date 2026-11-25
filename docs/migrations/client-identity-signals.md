# Subject-scoped identity signals and delimited web storage keys

This release changes how `@baukit/data-contracts` and `@baukit/api-runtime`
handle sign-out, erasure and 401 signals. A stale signal from one account could
erase another account's partition. Now every signal names its subject.

## ScopedPersistenceLifecycle

`handleProfileErased()`, `handleSessionExpired()` and
`blockIdentityMismatch(error)` now take the subject:

```ts
await lifecycle.handleProfileErased(subject);
await lifecycle.handleSessionExpired(subject);
await lifecycle.blockIdentityMismatch(partition.subject, error);
```

Each call is a no-op while another subject is selected or ready. Pass the
profile ID of the session that expired. The authenticated templates keep the
last confirmed profile ID for this.

## api-runtime

- `/path` now stays under the base path. With base `https://api.example.test/v1`,
  `fetch('/items')` goes to `/v1/items`, not `/items`. Callers that relied on
  the old drop must pass an absolute URL.
- The runtime adds `Authorization` and runs `onUnauthorized`, the 401 replay
  and `onProfileErased` only for the base origin under the base path.
  `//host`, `/\host`, other origins and `Request` objects outside the base get no
  token and fire no hook.
- New option `sessionSubject: () => string | undefined`. The runtime reads it at
  send time and again on a 401, and drops the signal if it changed. Hook
  contexts carry the stamped `subject`.

## Product-profile erasure

A failing `beforeServerErase` hook now aborts the erase with
`{ status: 'server-failure', error: { stage: 'before-server' } }` instead of
adding a warning and calling the server. The shared fixture case
`warnings-only` is now `before-server-failure`. Pass `erasureMarker` to
`eraseProductProfile` and the same store to `ScopedPersistenceLifecycle` so a
crash between the server erase and the local delete finishes the delete on the
next start.

## WebStorageKeyValueStore

Stored keys changed from `namespace + key` to `namespace + "\u001f" + key`.
Existing values are invisible until you migrate them. Construct the store with
`{ migrateLegacyKeys: true }` to move them on first use. Only do that if no
other namespace of the origin starts with this one (`drafts:u1` and
`drafts:u10`), because the old format cannot tell those apart. In that case,
drop the old drafts or move them by hand. Namespaces must not contain U+001F.
