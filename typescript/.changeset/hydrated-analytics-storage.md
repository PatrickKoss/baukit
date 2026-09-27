---
'@baukit/analytics-core': minor
'@baukit/analytics-posthog-native': minor
---

Add `HydratedAnalyticsStorage` to the new `@baukit/analytics-posthog-native/storage` subpath. It loads a list of keys from an asynchronous store such as AsyncStorage once, then serves the synchronous `AnalyticsStorage` port from memory and writes those keys through. A key that fails to read starts absent without dropping the others. The subpath does not import `posthog-react-native`.

Add `analyticsStorageKeys(prefix)` to `@baukit/analytics-core`. It returns the consent, anonymous ID, user ID, and alias guard keys that `AnalyticsClient` uses.

`posthog-react-native` is now an optional peer of `@baukit/analytics-posthog-native`. Apps that use the transport must install it themselves, which the setup instructions already said.

The mobile template now calls `HydratedAnalyticsStorage` instead of generating its own class. Products that copied the template class can delete it.
