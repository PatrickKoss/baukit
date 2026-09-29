---
'@baukit/data-contracts': patch
---

Add `WebStorageKeyValueStore`, a `KeyValueStore` over `sessionStorage` or `localStorage`. It keeps every key under a required non-empty namespace, stores JSON text, rejects instead of throwing on storage failures, and maps a full storage to `StorageError` `storage_quota_exceeded`. It passes `describeKeyValueContract`. No breaking changes.
