# @baukit/a11y-core

## 0.5.1

### Patch Changes

- 5a29c63: `@baukit/a11y-core/web` exports `useReducedMotion`, `useReducedMotionPreference`, and `ReducedMotionPreference`. The web versions use `matchMedia` and `useSyncExternalStore` and never import `react-native`, so a plain React web app can use them. They return the same shapes as the root hooks. During server rendering and hydration `useReducedMotionPreference` reports `{ reducedMotion: false, resolved: false }`. The root entry's hooks are unchanged.
- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 868b67e: Add headless menu navigation. `useRovingMenu({ active, options })` gives an action menu one tab stop on an enabled item, arrow-key movement that skips disabled items and wraps, and Home and End. It returns `itemProps(index)`, `activeIndex`, and a stable `initialFocusRef` to pass to `useOverlayA11y`, which keeps Escape, Tab containment, and focus restoration. With no enabled item there is no item tab stop and the overlay focuses its first focusable control. When the active item is removed or disabled while open, the tab stop moves to the item now at that position, or back to the initial item; focus follows if it fell to the document body. Keys from hosts nested inside an item are ignored. The pure `nextEnabledMenuIndex(key, currentIndex, options)` is exported for container-level key handlers. Both are available from the package root and from `@baukit/a11y-core/web`.

  `useOverlayA11y` now accepts any `HostRef` as `initialFocusRef`, matching `useFocusTrap`. Existing `RefObject<View | null>` values still type-check.

  No breaking changes.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- f249d7c: Add `focusAccessibilityElement(ref)` to the package root. On native it resolves the view tag with `findNodeHandle` and calls `AccessibilityInfo.setAccessibilityFocus`. On web it focuses the element with `preventScroll`. It returns false when there is nothing to focus. `useOverlayA11y` now shares its view tag lookup.

  The mobile template's `useRouteHeadingFocus` now moves native screen-reader focus to the route heading one frame after the route gains focus. It previously did nothing on native.

  No breaking changes.

- Release the coordinated baukit 0.5.0 train.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Add route-heading focus recovery for client-side navigation.
- Expose reduced-motion readiness so applications can avoid rendering before
  the user's motion preference is known.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/a11y-core`.
