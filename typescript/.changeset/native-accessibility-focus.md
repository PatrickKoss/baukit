---
'@baukit/a11y-core': minor
---

Add `focusAccessibilityElement(ref)` to the package root. On native it resolves the view tag with `findNodeHandle` and calls `AccessibilityInfo.setAccessibilityFocus`. On web it focuses the element with `preventScroll`. It returns false when there is nothing to focus. `useOverlayA11y` now shares its view tag lookup.

The mobile template's `useRouteHeadingFocus` now moves native screen-reader focus to the route heading one frame after the route gains focus. It previously did nothing on native.

No breaking changes.
