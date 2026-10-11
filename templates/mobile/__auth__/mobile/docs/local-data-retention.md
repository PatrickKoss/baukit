# Local data retention

This generated product uses **retain** as its logout policy. Logout unmounts user-scoped work, closes SQLite, resets in-memory state and analytics identity, then keeps the partition for the same server profile to reopen later.

The partition key is `CurrentUser.id` from `/me`, not the identity provider's subject. A product that keeps the identity account after erasure can admit the same subject after fresh authentication. Its new profile has a new ID, so even a device that missed the erasure opens a fresh database without restoring old data or an old outbox. The template confirms the profile before opening SQLite and rejects a `/me` response for another auth subject. Identity-scoped preference records also use the server profile ID.

The profile-to-database registry is versioned and stored in Expo SecureStore, outside the domain database. A corrupt registry blocks initialization; it never falls back to a shared database. Database filenames contain a SHA-256 digest, not the profile ID. Expo Crypto supplies SHA-256 because React Native does not expose Web Crypto by default.

Before adding a legacy database, provide an explicit ownership inspector and document its evidence. Do not claim empty, ambiguous or differently owned data. Before sync adoption or pushing an outbox, compare the server profile ID with the partition's key through `recheckServerSubjectBeforeSyncAdoption`. The helper calls that key `subject`; pass the profile ID on both sides.

A `session-expired` event with reason `profile_erased` closes and deletes the active database, removes its registry entry, and blocks local data. Other expiry reasons only close and block. Token clearing can close the partition before the reason arrives, so the template keeps its cleanup context until erasure finishes. Cleanup failures remain visible and preserve the registry entry.

A device that never receives `profile_erased` keeps the old database on disk. It cannot reopen it under the new profile ID. The product must clean up partitions of unknown profiles to remove that data. `LocalDataRetention` names logout policies; it does not implement cleanup. Data-contracts can erase an active partition and remove a known registry entry, but has no public API to enumerate entries or purge unknown profiles. Products need their own partition inventory and cleanup policy.

If logout changes to `delete`, close first, delete the database and active registry entry, preserve legacy-claim tombstones, and report deletion failures. For `quarantine`, keep the partition out of normal mounting until an explicit recovery or deletion flow exists. Add product tests for the selected policy.
