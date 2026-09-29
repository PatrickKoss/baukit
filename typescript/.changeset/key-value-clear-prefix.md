---
'@baukit/data-contracts': patch
'@baukit/data-contracts-dexie': patch
'@baukit/data-contracts-expo-sqlite': patch
---

Add `KeyValueStore.clearPrefix(prefix)`, which deletes every key that starts with `prefix`. Matching is exact and case-sensitive, no character is a wildcard, and an empty prefix clears the store. `InMemoryKeyValueStore`, `DexieKeyValueStore` (one IndexedDB key range), and the Expo SQLite key-value store (a UTF-8 byte prefix match inside the store's namespace) implement it, and `describeKeyValueContract` checks it, including SQL `LIKE` wildcards, case, emoji, and U+FFFF. The Dexie real-browser suite now runs the key-value contract too.

Breaking: `KeyValueStore` gains a required method, so a product's own `KeyValueStore` implementation must add `clearPrefix`.
