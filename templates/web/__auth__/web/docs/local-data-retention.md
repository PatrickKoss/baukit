# Local data retention

This generated product uses **retain** as its logout policy. Logout disables user-scoped queries, resets the query cache and analytics identity, and leaves any opaque per-subject browser partition for the same OIDC `sub` to reopen later.

The versioned subject-to-store registry lives in localStorage, outside any future Dexie/domain database. Corrupt metadata blocks initialization; it never falls back to a shared cache. Store names use Web Crypto SHA-256 and do not disclose the subject. Older browsers need a standards-compatible `crypto.subtle` polyfill before authenticated local data can initialize.

The starter has only an in-memory TanStack Query cache. If you add Dexie, open the derived `storeName` inside `src/local-data.ts` and close it from the returned partition. A restored browser session remains fail-closed until the backend confirms its subject; do not infer identity by decoding an unverified token locally.

Before sync adoption or an outbox push, recheck the server-confirmed subject with `recheckServerSubjectBeforeSyncAdoption`. If the policy changes to `delete` or `quarantine`, implement and test the corresponding registry and database handling without erasing legacy-claim tombstones.
