# `@baukit/a11y-core`

`@baukit/a11y-core` holds the accessibility behavior that web and React Native products share:
overlay focus, inert background content, announcements, reduced motion, and keyboard movement
through groups and forms. Products keep their components, copy, and visual design local.

The package imports React and React Native and nothing else. Both are peer dependencies, so the
product's Expo SDK decides the versions, and React Native is optional.

## Two entry points

A React Native product imports the package root and gets everything. A plain React web app imports
`@baukit/a11y-core/web` and gets `useFocusTrap`, `useInert`, `useAriaHiddenInert`,
`useReducedMotion`, `useReducedMotionPreference`, `useRovingMenu`, `nextEnabledMenuIndex`,
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

## Menus

`useRovingMenu` holds the keyboard state of an action menu. It gives the menu one tab stop on an
enabled item, moves focus with the arrow keys while skipping disabled items, wraps at both ends,
and sends Home and End to the first and last enabled items. It renders nothing and never invokes an
action. Escape, Tab containment, the inert background, and focus restoration stay with
`useOverlayA11y`, so call `useRovingMenu` first and pass its `initialFocusRef` on:

```tsx
const menu = useRovingMenu({ active: visible, options: items });
const overlay = useOverlayA11y({
  active: visible,
  containerRef: panelRef,
  initialFocusRef: menu.initialFocusRef,
  onEscape: onClose,
  triggerRef,
});

items.map((item, index) => (
  <Pressable key={item.id} {...menu.itemProps(index)} disabled={item.disabled} />
));
```

`options` only needs `disabled` and `selected`, so the product's item array fits as is. The tab
stop starts on the first selected enabled item, otherwise the first enabled item, and returns there
each time the menu opens. `itemProps(index)` returns the `tabIndex`, `ref`, and `onKeyDown` for one
item; `activeIndex` names the item holding the tab stop.

- With no enabled item, `activeIndex` is null, every item has `tabIndex: -1`, arrow keys do
  nothing, and `initialFocusRef.current` is null. `useOverlayA11y` then focuses the first focusable
  element in the container, so put the close control before the items.
- When the active item is disabled or removed while the menu is open, the tab stop moves to the
  item now at that position if it is enabled, otherwise back to the initial item. If the removed
  item held focus and focus fell to the document body, the hook focuses the new active item.
- A key that comes from a host nested inside an item, such as a text field, is left alone.
- `initialFocusRef` is one stable ref for the life of the component. The hook points it at the
  active item in an effect, and the overlay reads it in a later effect when the menu opens, so both
  hooks must run in the same component with `useRovingMenu` first. Do not swap it for another ref
  while the menu is open: the web trap would re-enter and forget the trigger it restores to.

`nextEnabledMenuIndex(key, currentIndex, options)` is the pure step behind the hook, for a product
that handles keys on the menu container instead. It returns null for keys other than the arrows,
Home, and End, and for a menu without an enabled item. It moves relative to `currentIndex` even
when that item is disabled or gone: an index past the end moves Down to the first enabled item and
Up to the last, and so does `-1`.

Action order, closing before or after an action, async failure recovery, selection meaning, and
copy stay with the product.

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

React Native has no document, so the controller does not apply there. To move VoiceOver or
TalkBack to a destination heading, call `focusAccessibilityElement(headingRef)` from the root
entry once the heading has laid out, for example one animation frame after the route gains focus.
On native it resolves the view tag and calls `AccessibilityInfo.setAccessibilityFocus`. On web it
focuses the element with `preventScroll`. It returns false when the ref is empty, the view is
unmounted, or the web element cannot take focus.

```ts
import { focusAccessibilityElement } from '@baukit/a11y-core';

const frame = requestAnimationFrame(() => {
  focusAccessibilityElement(headingRef);
});
```

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

The web entry exports both hooks with the same names and return shapes, built on `matchMedia` and
`useSyncExternalStore` alone. They read the media query during the first client render and follow
its `change` events. During server rendering and the hydration render they report
`{ reducedMotion: false, resolved: false }`, then resolve on the client. A host without
`matchMedia` reports no preference, resolved.

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
