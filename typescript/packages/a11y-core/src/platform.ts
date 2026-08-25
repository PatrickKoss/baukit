/**
 * The DOM-only hooks must be importable from a plain React web app that has no
 * React Native in its dependency tree. A static `react-native` import would put
 * React Native's Flow source in that app's bundler, so those hooks ask for a
 * document instead of asking `Platform` which OS this is.
 *
 * The two questions have the same answer for this package. Every branch guarded
 * by `Platform.OS === 'web'` in `use-focus-trap` and `use-inert` reads or writes
 * the DOM, React Native Web runs them against a real document, and React Native
 * has no document at all. Hooks that need a genuine platform split, such as
 * `announce` and `useOverlayA11y`, keep importing `Platform` directly.
 */

/** True when a DOM document is available to read and mutate. */
export function hasDocument(): boolean {
  return typeof document !== 'undefined';
}
