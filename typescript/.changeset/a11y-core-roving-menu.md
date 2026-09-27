---
'@baukit/a11y-core': minor
---

Add headless menu navigation. `useRovingMenu({ active, options })` gives an action menu one tab stop on an enabled item, arrow-key movement that skips disabled items and wraps, and Home and End. It returns `itemProps(index)`, `activeIndex`, and a stable `initialFocusRef` to pass to `useOverlayA11y`, which keeps Escape, Tab containment, and focus restoration. With no enabled item there is no item tab stop and the overlay focuses its first focusable control. When the active item is removed or disabled while open, the tab stop moves to the item now at that position, or back to the initial item; focus follows if it fell to the document body. Keys from hosts nested inside an item are ignored. The pure `nextEnabledMenuIndex(key, currentIndex, options)` is exported for container-level key handlers. Both are available from the package root and from `@baukit/a11y-core/web`.

`useOverlayA11y` now accepts any `HostRef` as `initialFocusRef`, matching `useFocusTrap`. Existing `RefObject<View | null>` values still type-check.

No breaking changes.
