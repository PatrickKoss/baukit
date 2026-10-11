---
'@baukit/data-contracts': minor
---

Add `ScopedPersistenceLifecycle.handleProfileErased()` and blocked reason `profile-erased`. Erase the last selected partition even after session cleanup closes it, remove its registry entry, and keep cleanup errors in the blocked state. Serialize erasure with identity transitions so a concurrent selection cannot reopen the erased profile.
