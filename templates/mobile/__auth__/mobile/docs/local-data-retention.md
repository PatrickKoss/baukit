# Local data retention

This generated product uses **retain** as its logout policy. Logout unmounts user-scoped work, closes SQLite, resets in-memory state and analytics identity, then leaves the opaque per-subject database on the device. It can be reopened only after the same immutable OIDC `sub` signs in again.

The subject-to-database registry is versioned and stored in Expo SecureStore, outside the domain database. A corrupt registry blocks initialization; it never falls back to a shared database. Database filenames contain a SHA-256 digest, not the subject. Expo Crypto supplies SHA-256 because React Native does not expose Web Crypto by default.

Before adding a legacy database, provide an explicit ownership inspector and document its evidence. Do not claim empty, ambiguous or differently owned data. Before sync adoption or pushing an outbox, recheck the server-confirmed subject with `recheckServerSubjectBeforeSyncAdoption`.

If the policy changes to `delete`, close first, delete the subject database and active registry entry, preserve legacy-claim tombstones, and surface deletion failures. For `quarantine`, remove the partition from normal mounting until an explicit recovery or deletion flow exists. Add product tests for the selected policy.
