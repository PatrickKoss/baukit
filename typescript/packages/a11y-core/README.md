# `@baukit/a11y-core`

`@baukit/a11y-core` holds the accessibility behavior that web and React Native products share:
overlay focus, inert background content, announcements, reduced motion, and keyboard movement
through groups and forms. Products keep their components, copy, and visual design local.

The package imports React and React Native and nothing else. Both are peer dependencies, so the
product's Expo SDK decides the versions, and React Native is optional.

## Two entry points

A React Native product imports the package root and gets everything. A plain React web app imports
`@baukit/a11y-core/web` and gets `useFocusTrap`, `useInert`, `useAriaHiddenInert`,
`useSingleFlight`, `createRouteFocusController`, and the `dom-boundary` helpers. Nothing reachable
from that entry imports `react-native`, at runtime or in its types, so the app needs no React
Native in its dependency tree. `react-native` is an optional peer dependency for exactly that
reason.

The DOM hooks ask whether a document exists rather than asking `Platform` which OS this is. The
two questions have the same answer here: every branch those hooks guard reads or writes the DOM,
React Native Web gives them a real document, and React Native has none. Hooks with a genuine
platform split, such as `announce` and `useOverlayA11y`, keep reading `Platform` and stay behind
the root entry.

## Overlays

`useOverlayA11y` is one contract over two platforms that behave nothing alike.

```ts
const { backgroundProps, containerProps } = useOverlayA11y({
  active: visible,
  containerRef: panelRef,
  inertContainerRef: overlayRootRef,
  initialFocusRef,
  onEscape: onClose,
  triggerRef,
});
```

On web it moves focus into the overlay, contains Tab and Shift+Tab, calls `onEscape`, makes
everything outside `inertContainerRef` inert, and restores focus to whatever had it before. The
restore runs on the next task because the browser rejects focus on a still-inert trigger.

On native it waits before requesting accessibility focus, so no focus lands before layout. On
close it returns focus to `triggerRef`. Native cannot discover what previously held accessibility
focus, so the caller owns that ref. Pass `triggerHandle` instead when the node tag is already
resolved.

The wait defaults to `InteractionManager.runAfterInteractions`, which React Native 0.86 deprecates
without offering a replacement for "after the overlay presented". Products with a real
presentation event should pass `deferFocus` and drive the focus request from it:

```ts
const deferFocus = (task: () => void) => {
  layoutTasks.push(task);
  return { cancel: () => remove(task) };
};
```

Spread `backgroundProps` onto the content _behind_ the overlay, never the overlay itself. It sets
`accessibilityElementsHidden` and `importantForAccessibility` while the overlay is open, and is
empty on web and while closed. Spread `containerProps` onto the overlay container.

`useFocusTrap` and `useInert` are the web halves on their own, for products that compose their own
overlay. `useAriaHiddenInert` covers a separate problem: routers mark inactive web scenes
`aria-hidden`, which leaves their descendants in the keyboard focus order. Mount it once at the
app root. Put `ARIA_HIDDEN_INERT_OPT_OUT` on an element that must stay focusable anyway.

## Route focus

WebKit can blur the initiating control to `body` when a router makes the outgoing scene inert.
Create one `createRouteFocusController` at app startup so it can remember the last reachable
focused element. Enter a route with a target getter, then run the returned cleanup when the route
becomes inactive:

```ts
const routeFocus = createRouteFocusController();

const leaveRoute = routeFocus.enterRoute(() =>
  document.querySelector<HTMLElement>('[data-route-heading]'),
);

leaveRoute();
routeFocus.dispose();
```

The controller waits for the destination target to mount and retries on animation frames for up to
1.5 seconds. It waits for inert or hidden return targets to become reachable. It stops if the user
moves focus to another reachable element, and it never focuses a target below `inert`,
`aria-hidden="true"`, or `hidden`.

## The React Native to DOM boundary

React Native Web renders a `View` as a DOM element, but the `View` type never says so. Every
crossing goes through `dom-boundary`, which checks for the method it needs and returns null
otherwise. `asFocusTarget`, `asFocusContainer`, and `asTreeElement` take a `HostRef` and narrow
it; a native host that has no DOM capability simply yields null instead of throwing. `HostRef` is
`RefObject<object | null>`, which accepts a `View` ref and an element ref without naming either
type.

## Announcements

`announce(message, options)` speaks an outcome without a visible live-region component. Native
calls `announceForAccessibility`. Web writes into a visually hidden region, clearing the text and
forcing a reflow first so the same message twice is spoken twice. Blank messages are dropped.

The region's DOM id defaults to `baukit-announcer`. Pass `liveRegionId` to place it under a
product-owned id, and `assertive: true` to interrupt rather than wait for a pause.

## Reduced motion, groups, and forms

`useReducedMotionPreference` returns `{ reducedMotion, resolved }` and follows preference changes
on both platforms. Web resolves during the first render. The native query is asynchronous, so
`resolved` starts as `false` and becomes `true` whether the query succeeds or fails. Do not start
non-essential motion until it is `true`. `useReducedMotion` remains the boolean form for existing
callers.

`useRovingRadioGroup` gives a radio group a single tab stop and arrow-key movement between its
options, wrapping at both ends and honoring Home and End. `radioProps(index)` returns the
`tabIndex`, `ref`, and `onKeyDown` for one option.

`useEnterToNext` makes Enter walk a web form field by field and submit from the last one.
Multiline fields keep their newline behavior and are skipped along the way. On native it returns
only the `ref`, leaving the platform keyboard alone.

`useSingleFlight` is a synchronous mutex for async UI mutations. React state cannot lock within a
single tick, so a double tap would submit twice. A rejected call resolves to `undefined`, and the
lock is released even when the operation throws.

## Boundaries

This package renders nothing and ships no components, styles, or copy. It does not decide which
outcomes deserve an announcement, which motion is essential, or what an overlay looks like. Layout
breakpoint arithmetic lives in `@baukit/ui-tokens`, not here.
