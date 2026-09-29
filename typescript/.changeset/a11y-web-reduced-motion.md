---
'@baukit/a11y-core': patch
---

`@baukit/a11y-core/web` exports `useReducedMotion`, `useReducedMotionPreference`, and `ReducedMotionPreference`. The web versions use `matchMedia` and `useSyncExternalStore` and never import `react-native`, so a plain React web app can use them. They return the same shapes as the root hooks. During server rendering and hydration `useReducedMotionPreference` reports `{ reducedMotion: false, resolved: false }`. The root entry's hooks are unchanged.
