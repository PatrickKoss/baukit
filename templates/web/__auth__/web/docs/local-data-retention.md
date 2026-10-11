# Local data retention

This generated product uses **retain** as its logout policy. Logout disables user-scoped queries, resets the query cache and analytics identity, and keeps the partition for the same server profile to reopen later.

The partition key is `CurrentUser.id` from `/me`, not the identity provider's subject. A product that keeps the identity account after erasure can admit the same subject after fresh authentication. Its new profile has a new ID, so this device opens a fresh partition without restoring the erased profile's data or outbox. Confirm the profile on the server before opening local data.

The versioned profile-to-store registry lives in localStorage, outside any future Dexie/domain database. Corrupt metadata blocks initialization; it never falls back to a shared cache. Store names use Web Crypto SHA-256 and do not disclose the profile ID. Older browsers need a standards-compatible `crypto.subtle` polyfill before authenticated local data can initialize.

The starter has only an in-memory TanStack Query cache. If you add Dexie, open the derived `storeName` inside `src/local-data.ts`, close it from the returned partition, and delete it in the shared `erasePartition` callback. Before sync adoption or an outbox push, compare the server profile ID with the partition's key through `recheckServerSubjectBeforeSyncAdoption`. The helper calls that key `subject`; pass the profile ID on both sides.

A `session-expired` event with reason `profile_erased` blocks local access, erases the active partition, and removes its registry entry. Other expiry reasons only close and block. Token clearing can close the partition before the reason arrives, so the template keeps its cleanup context until erasure finishes. Cleanup failures remain visible and preserve the registry entry.

A device that never receives `profile_erased` keeps the old partition on disk. It cannot reopen it under the new profile ID. The product must clean up partitions of unknown profiles to remove that data. `LocalDataRetention` names logout policies; it does not implement cleanup. Data-contracts can erase an active partition and remove a known registry entry, but has no public API to enumerate entries or purge unknown profiles. Products need their own partition inventory and cleanup policy. Preserve legacy-claim tombstones when removing registry entries.
