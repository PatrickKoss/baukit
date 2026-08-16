# Authenticated local-data ownership contract

**Status:** Contract and provider-neutral helpers shipped in `@baukit/data-contracts`.
**Applies to:** Every authenticated product that persists user-scoped data on a device.
**Related:** [offline readiness](./offline-readiness-contract.md) and [analytics identity](./analytics-privacy-contract.md).

This is a security boundary. A sync-time identity check alone is insufficient: the wrong account must not be able to read or mutate another account's local partition before sync starts.

## 1. Partition identity

- The partition key is the exact immutable OIDC `sub` from the validated session. Email, display name, an analytics ID, and a mutable product user ID are not partition keys.
- Combine the subject with a stable application namespace using an unambiguous canonical encoding, then derive a deterministic, collision-resistant store name with a one-way digest. The resulting SQLite, IndexedDB, cache, or filesystem name must not disclose the subject.
- Keep the subject-to-store mapping in a versioned registry outside the domain database. Serialize registry reads and writes so concurrent account selection cannot claim one store twice.
- Treat an empty subject, an invalid digest result, an unsupported registry version, malformed registry data, and inconsistent ownership metadata as blocking initialization failures. Never fall back to a shared or legacy store.

Mapping an OIDC subject to an internal server UUID remains a backend concern. Product database schemas and repository types do not belong in this contract.

## 2. Legacy stores

A product may expose an explicit legacy-store inspector during migration. A legacy store may be claimed at most once and only when one of these rules proves ownership:

- persisted ownership already names the current OIDC subject; or
- every legacy owner marker is the product's documented pre-authentication placeholder and no server identity has adopted the store.

An empty, ambiguous, or differently owned legacy store is not evidence for a claim. Leave it closed and create a fresh scoped partition. Record the successful claim in the registry before allowing normal use. A product may quarantine an unclaimable store, but must not silently merge it.

## 3. Identity transition

On login, logout, token-subject change, or account switch, perform this order:

1. Unmount user-scoped repositories and stop sync/query work.
2. Close the active database and wait for closure before opening another partition.
3. Reset all user-scoped in-memory state, including sync status, pending mutation views, query caches, locale/theme preferences, and analytics identity.
4. Resolve and validate the registry entry for the new `sub`.
5. Open the partition and complete migrations and local hydration.
6. Mount product repositories only after the partition is ready and still belongs to the active subject.
7. Before adopting a server identity or pushing pending mutations, recheck that the server-confirmed subject matches the active partition subject. On mismatch, stop sync and enter a blocking identity-mismatch state.

Late work from a previous partition must be cancelled or ignored. UI below the repository provider must not render against a stale handle while initialization is pending or blocked.

## 4. Logout retention

Every product must choose and document one policy per data class:

- `retain`: close the partition but keep it for a later login by the same subject;
- `delete`: close it, remove its data and active registry entry, preserve any legacy-claim tombstone, and surface deletion failure honestly; or
- `quarantine`: make it unavailable to normal mounting until an explicit recovery or deletion flow.

Logout always performs the close and in-memory reset regardless of retention. Retention never permits a different subject to mount the partition.

## 5. Shipped helper shape

`@baukit/data-contracts` exports these provider-neutral seams:

```ts
type LocalDataRetention = "retain" | "delete" | "quarantine";

interface ScopedPersistenceRegistryStore {
  read(): Promise<string | null>;
  write(serialized: string): Promise<void>;
}

type LegacyStoreInspection =
  | { exists: false }
  | {
      exists: true;
      ownership: "claimable" | "current-subject" | "other-subject" | "ambiguous";
    };

interface ResolveScopedStoreOptions {
  namespace: string;
  subject: string;
  registry: ScopedPersistenceRegistryStore;
  inspectLegacy?: (subject: string) => Promise<LegacyStoreInspection>;
  legacyStoreName?: string;
}

declare function deriveScopedStoreName(
  namespace: string,
  subject: string,
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

The exact implementation additionally exports `InMemoryScopedPersistenceRegistryStore`,
`ScopedPersistenceLifecycle`, `recheckServerSubjectBeforeSyncAdoption`, and the
adapter-parameterized `describeScopedPersistenceContract` Vitest suite. The
lifecycle has explicit signed-out, initializing, ready, and blocked states. It
hides an old handle immediately, waits for close and user-memory reset, and
publishes the new partition only after the injected open/migrate/hydrate hook
finishes. Terminal `subscribeSessionExpired` events call
`handleSessionExpired()`: close and block, never a fabricated subject switch.

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

## 6. Acceptance checks

- E signs in, writes records and pending mutations, signs out; F signs in and can neither read nor modify E's data; E returns and sees only E's original data and pending mutations.
- E→F→E waits for close before each open and resets every named in-memory store and query/analytics identity.
- Web and native adapters pass the same transition suite, including an offline account switch.
- Corrupt, truncated, unknown-version, and semantically inconsistent registries fail closed without opening any domain store.
- Concurrent first login allows at most one subject to claim the legacy store.
- An unowned-placeholder legacy store can be claimed once; a same-subject store can be reclaimed; an empty, ambiguous, or other-subject store cannot be claimed.
- Product repositories remain unmounted until migration and hydration succeed.
- A server-subject mismatch blocks sync adoption and leaves local data unchanged.
- Retain, delete, or quarantine behavior is tested for the product's declared logout policy.
