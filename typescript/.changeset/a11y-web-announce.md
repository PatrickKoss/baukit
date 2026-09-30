---
'@baukit/a11y-core': patch
---

`@baukit/a11y-core/web` exports `announce(message, { assertive, liveRegionId })`, `AnnounceOptions`, and `DEFAULT_LIVE_REGION_ID`, so a plain React web app can speak outcomes through a visually hidden ARIA live region without React Native in its dependency tree. The call shape and the region match the root export, which now delegates to the same web module on React Native Web.
