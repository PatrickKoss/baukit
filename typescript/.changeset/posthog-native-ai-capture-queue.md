---
'@baukit/analytics-posthog-native': minor
---

The consent purge now also clears `ai_capture_queue`, the persisted queue that `posthog-react-native` 4.78 (through `@posthog/core` 1.55) added. Before this, events queued there survived a consent withdrawal. `PostHogNativeClient.setPersistedProperty` takes the key as the string union `` `${PostHogPersistedProperty}` ``, so fakes can pass plain strings. The `posthog-react-native` peer floor is now `^4.78.3`.
