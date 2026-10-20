---
'@baukit/a11y-core': patch
---

Keep focus moved outside a closing overlay and avoid disconnected openers. Add `restoreFocus` and
`returnFocus` callbacks to `useFocusTrap` and `useOverlayA11y` for opting out or choosing a fallback.
