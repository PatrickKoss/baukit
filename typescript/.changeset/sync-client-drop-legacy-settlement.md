---
'@baukit/sync-client': minor
---

Breaking: the sync snapshot drops `lastSyncAt`, `setError` and the positional `hydrate` overload are gone, and conformance adapters must provide `pendingId` and `applySubmittedBatchOutcome`. `markAcknowledged` and `recordRejected` are no longer called; delete them from adapters.
