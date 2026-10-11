# Authenticated local-data ownership contract

**Status:** Contract and provider-neutral helpers shipped in `@baukit/data-contracts`.
**Applies to:** Every authenticated product that persists user-scoped data on a device.
**Related:** [offline readiness](./offline-readiness-contract.md) and [analytics identity](./analytics-privacy-contract.md).

This is a security boundary. A sync-time identity check alone is insufficient: the wrong account must not be able to read or mutate another account's local partition before sync starts.

## 1. Partition identity

- The partition key is the server profile ID confirmed by `/me`. It stays immutable for that profile and changes when erasure is followed by re-onboarding. Products that keep the identity account must not use the OIDC `sub`: the same subject can own a new profile. Email, display name, analytics IDs, and mutable user IDs are not partition keys.
- Combine the profile ID with a stable application namespace using an unambiguous canonical encoding, then derive a deterministic, collision-resistant store name with a one-way digest. The resulting SQLite, IndexedDB, cache, or filesystem name must not disclose the profile ID.
- Keep the profile-to-store mapping in a versioned registry outside the domain database. Serialize registry reads and writes so concurrent account selection cannot claim one store twice.
- Treat an empty profile ID, an invalid digest result, an unsupported registry version, malformed registry data, and inconsistent ownership metadata as blocking initialization failures. Never fall back to a shared or legacy store.

The backend maps a validated OIDC subject to its current profile ID. The client confirms that mapping before opening storage. Data-contracts names the partition key `subject`; products pass the profile ID to that parameter. Product schemas and repository types remain product code.

## 2. Legacy stores

A product may expose an explicit legacy-store inspector during migration. A legacy store may be claimed at most once and only when one of these rules proves ownership:

- persisted ownership already names the current server profile ID; or
- every legacy owner marker is the product's documented pre-authentication placeholder and no server identity has adopted the store.

An empty, ambiguous, or differently owned legacy store is not evidence for a claim. Leave it closed and create a fresh scoped partition. Record the successful claim in the registry before allowing normal use. A product may quarantine an unclaimable store, but must not silently merge it.

## 3. Identity transition

On login, logout, token-subject change, or account switch, perform this order:

1. Unmount user-scoped repositories and stop sync/query work.
2. Close the active database and wait for closure before opening another partition.
3. Reset all user-scoped in-memory state, including sync status, pending mutation views, query caches, locale/theme preferences, and analytics identity.
4. Resolve and validate the registry entry for the new profile ID.
5. Open the partition and complete migrations and local hydration.
6. Mount product repositories only after the partition is ready and still belongs to the active profile ID.
7. Before adopting a server identity or pushing pending mutations, recheck that the server-confirmed profile ID matches the active partition key. On mismatch, stop sync and enter a blocking identity-mismatch state.

Late work from a previous partition must be cancelled or ignored. UI below the repository provider must not render against a stale handle while initialization is pending or blocked.

## 4. Logout retention

Every product must choose and document one policy per data class:

- `retain`: close the partition but keep it for a later login by the same profile ID;
- `delete`: close it, remove its data and active registry entry, preserve any legacy-claim tombstone, and surface deletion failure honestly; or
- `quarantine`: make it unavailable to normal mounting until an explicit recovery or deletion flow.

Logout always performs the close and in-memory reset regardless of retention. Retention never permits a different profile to mount the partition.

## 5. Product-profile erasure

The identity-scoped persistence lifecycle supports explicit erasure of the active
partition after authoritative server success. Construct
`ScopedPersistenceLifecycle` with an `erase({ subject, storeName })` hook, then
call `eraseActivePartition()` inside the product-profile sequence's
`eraseLocalPartition` adapter. That adapter returns `Promise<void>`, so it must
also decide whether the lifecycle's `false` result is an expected no-op or a
local cleanup failure. The lifecycle method immediately hides a ready partition
and publishes `signed-out`, then runs the serialized transition: close the store,
reset user-scoped memory, call `erase`, and remove the registry mapping. It
resolves `true` after those steps finish. With no ready partition it resolves
`false`; with a ready partition but no configured `erase` hook it rejects.

If close, reset, or physical deletion fails, the registry entry remains. Registry
removal is the final step and its failure is returned to the caller. The exported
`removeScopedPersistenceRegistryEntry({ namespace, subject, registry })` performs
that locked removal directly and resolves `true` when it removes an entry or
`false` when no matching entry exists. Products use these APIs rather than read,
parse, edit, or rewrite the serialized registry JSON themselves.

Auth integration must pass the session-expired reason to local-data cleanup.
For `profile_erased`, block local access and erase the partition and its
registry entry. Other expiry reasons only close and block. The generated templates
also keep the last partition's cleanup context because native token clearing can
close storage before the expiry reason arrives.

A device that never receives `profile_erased` retains the old partition on disk.
A new profile ID prevents that partition's data and outbox from being adopted,
but physical removal needs a product cleanup policy for unknown profiles.
`LocalDataRetention` is a policy type, not an automatic cleanup service. The
registry retains entries for other IDs. Data-contracts can erase the active
partition and remove a known entry, but offers no public enumeration or bulk
cleanup API. Products need their own partition inventory and cleanup policy;
removing an entry alone does not delete its database.

## 6. Import safety

Treat every import file as untrusted input, even when the product created the export. Read a bounded
string or byte array, check its byte length before decoding, and stop row iteration as soon as it
exceeds the product's row limit. Apply a product-defined field allowlist before row decoding. Check
strings inside allowed fields against the product's byte limit so a nested object cannot bypass the
same bound used by a top-level field.

An import has two phases. Preparation decodes the product envelope, validates rows, and builds a
preview plan without receiving a write adapter. Commit passes that plan to one product transaction.
The product must not advance a cursor, publish a sync state, or request sync until the transaction
commits. A failed write rolls back the complete plan and leaves those states unchanged.

`@baukit/data-contracts/import-envelope` supplies `prepareImportEnvelope` and
`commitImportEnvelope`. The product supplies the envelope decoder, field allowlist, row decoder,
preview planner, transaction adapter, and post-commit callback. Schema versions, entities,
required fields, duplicate-ID handling, tombstone policy, conflict policy, provenance, deletion
order, and user copy remain product code. Source ownership, revision, and dirty-state fields should
usually be absent from the allowlist. The transaction should assign the active partition's
ownership and local sync metadata instead.

Do not log rejected source text, field values, or decoder errors that contain file content. Release
the source and prepared plan when the screen closes or the active identity changes. A preview can
become stale while another write runs, so the product must enforce its conflict policy again inside
the commit transaction.

Migration is additive. A product can first wrap its current parser and preview planner, then move
its existing write loop behind `commitImportEnvelope`. No file format, schema version, or storage
migration is required.

## 7. Shipped helper shape

`@baukit/data-contracts` exports these provider-neutral seams:

```ts
type LocalDataRetention = "retain" | "delete" | "quarantine";

interface ScopedPersistenceRegistryStore {
  read(): Promise<string | null>;
  write(serialized: string): Promise<void>;
}

type ScopedPersistenceDigest = (value: string) => Promise<string>;

type LegacyStoreInspection =
  | { exists: false }
  | {
      exists: true;
      ownership:
        "claimable" | "current-subject" | "other-subject" | "ambiguous";
    };

interface ResolveScopedStoreOptions {
  namespace: string;
  subject: string;
  registry: ScopedPersistenceRegistryStore;
  digest?: ScopedPersistenceDigest;
  inspectLegacy?: (subject: string) => Promise<LegacyStoreInspection>;
  legacyStoreName?: string;
}

declare function deriveScopedStoreName(
  namespace: string,
  subject: string,
  digest?: ScopedPersistenceDigest,
): Promise<string>;
declare function resolveScopedStore(
  options: ResolveScopedStoreOptions,
): Promise<{
  storeName: string;
  claimedLegacy: boolean;
}>;

declare class PersistenceIdentityMismatchError extends Error {
  readonly code: "persistence_identity_mismatch";
}
```

Other production exports used by this contract include
`InMemoryScopedPersistenceRegistryStore`, `ScopedPersistenceLifecycle`,
`removeScopedPersistenceRegistryEntry`,
`recheckServerSubjectBeforeSyncAdoption`, and
`isPersistenceIdentityMismatchError`. The lifecycle state statuses are
`signed-out`, `initializing`, `ready`, and `blocked`. Blocked reasons are
`identity-mismatch`, `initialization-failed`, and `session-expired`. The
lifecycle hides an old handle immediately, waits for close and user-memory
reset, and publishes the new partition only after the injected
open/migrate/hydrate hook finishes. Product auth integration calls
`eraseActivePartition()` for `profile_erased`, then `handleSessionExpired()`
to block access. Other terminal expiry reasons call `handleSessionExpired()`
directly. That method closes and blocks; it does not erase retained data.

The adapter-parameterized `describeScopedPersistenceContract` Vitest suite is
exported from the test-only `@baukit/data-contracts/vitest` subpath, not the
production entry point.

Browser and Node runtimes use `globalThis.crypto.subtle` for SHA-256. React
Native must install a compatible Web Crypto polyfill before using the default
digest or inject `ScopedPersistenceDigest`; the generated Expo composition uses
`expo-crypto`. The injected registry store should be backed by SecureStore,
localStorage, or an equivalent key/value service outside the domain database.
All callers sharing a registry must share one registry-store instance so the
in-process claim queue can serialize access. Cross-process products need their
backing store to provide the corresponding exclusive-write guarantee.

These seams have no dependency on React, OIDC SDKs, database adapters, product
user IDs, or product schemas. Products map their own legacy evidence into the
neutral ownership classification.
After a legacy claim, products must continue supplying the configured legacy
store name so the registry can verify it on every reopen. One store name may
belong to only one registry entry, including across namespaces.

## 8. Acceptance checks

- E signs in, writes records and pending mutations, signs out; F signs in and can neither read nor modify E's data; E returns and sees only E's original data and pending mutations.
- E→F→E waits for close before each open and resets every named in-memory store and query/analytics identity.
- Web and native adapters pass the same transition suite, including an offline account switch.
- Corrupt, truncated, unknown-version, and semantically inconsistent registries fail closed without opening any domain store.
- Concurrent first login allows at most one subject to claim the legacy store.
- An unowned-placeholder legacy store can be claimed once; a same-subject store can be reclaimed; an empty, ambiguous, or other-subject store cannot be claimed.
- Product repositories remain unmounted until migration and hydration succeed.
- A server-subject mismatch blocks sync adoption and leaves local data unchanged.
- Retain, delete, or quarantine behavior is tested for the product's declared logout policy.
- Active-partition erasure closes and resets before physical deletion, removes
  the matching registry entry only after deletion succeeds, and returns local
  deletion or registry failures to the caller.
- Import preparation rejects unsupported fields and configured limits without writing. A halfway
  commit failure leaves all rows, cursors, and sync state unchanged. A successful commit advances
  cursor or sync state only after every row is durable.
