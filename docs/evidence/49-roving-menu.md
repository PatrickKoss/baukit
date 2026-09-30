# Headless menu navigation evidence

Item 13 of the [cross-product feature plan](../cross-product-feature-plan.md). Steps 1 to 3 land
here. Step 4, product adoption, is listed at the end and has not started.

## Source revisions

- Eigenruhe `f74cebb`: `mobile/src/components/context-menu.tsx:64-69` (enabled, selected, and
  initial indexes, item refs, active index state), `:83-87` (reset on close), `:137-166` (key
  handler), item `ref` and `tabIndex` in the item map.
- Hebkit `841bf5d`: `mobile/src/components/context-menu.tsx:57-62`, `:77-78`, `:151-179`, and
  `mobile/src/components/context-menu.test.tsx:193-283` (first enabled item focus, Arrow keys over
  a disabled middle item, Escape).
- Tiefgang `2d37a06`: `mobile/src/components/context-menu.tsx:79-84`, `:99-103`, `:159-200`.
- Redemut `a782538`: `packages/ui/src/context-menu.tsx:49-60` (initial focus and trigger
  restoration) and `:81-114` (container key handler), `packages/ui/test/components.test.tsx:94-146`
  (disabled first item, selected item, End, ArrowUp, ArrowRight, Home, Space, Escape).
- Solo Leveling System `3461eaf`: `mobile/src/components/context-menu.tsx:56-106` (hand-rolled
  Tab and Escape trap, restoration) and `:150-180` (items).
- Baukit study: `docs/studies/31-expo-ui-and-headless-accessibility.md`.

Eigenruhe, Hebkit, and Tiefgang carry the same key handler line for line. Redemut does the same
work on the DOM from the menu container, reading `document.activeElement`. Solo Leveling System has
no arrow-key movement at all: every enabled item is its own Tab stop.

## Baukit owner

`@baukit/a11y-core` (`typescript/packages/a11y-core/src/use-roving-menu.ts`).

## Public types

- `nextEnabledMenuIndex(key: string, currentIndex: number, options: readonly RovingMenuOption[]):
  number | null`.
- `useRovingMenu({ active, options }: RovingMenuOptions): RovingMenuResult`, where the result is
  `{ activeIndex: number | null; initialFocusRef: RefObject<object | null>; itemProps(index):
  RovingMenuItemProps }` and item props are `{ onKeyDown, ref, tabIndex: 0 | -1 }`.
- `RovingMenuOption` (`disabled?`, `selected?`), `RovingMenuOptions`, `RovingMenuKeyEvent`,
  `RovingMenuItemProps`, `RovingMenuResult`.
- `OverlayA11yOptions.initialFocusRef` widens from `RefObject<View | null>` to `HostRef`, the type
  `useFocusTrap` already takes. Every existing value still type-checks.

Both functions ship from the package root and from `@baukit/a11y-core/web`. The module imports
nothing from React Native, at runtime or in its types, and `web.test.ts` fails if the web entry
reaches `react-native`.

## Behavior decisions

The pure function:

- Arrow Down and Arrow Right move to the next enabled item after `currentIndex` and wrap to the
  first enabled item. Arrow Up and Arrow Left move to the previous enabled item and wrap to the
  last. Every surveyed product maps Left and Right the same way, so the hook keeps them.
- Home and End go to the first and last enabled items.
- Movement is relative to the index, not to a position in the enabled list. A disabled current
  item moves to its enabled neighbour. A current index past the end (the item was removed) and
  `-1` both move Down to the first enabled item and Up to the last. The products use
  `Math.max(0, position) + 1` on the enabled list, which skips the first enabled item when the
  current item is not in that list.
- No enabled items, or an empty list, returns null for every key. Any other key returns null.

The hook:

- The tab stop starts on the first selected enabled item, otherwise the first enabled item. A
  selected item that is disabled is skipped. This matches all three RN products and Redemut.
- The tab stop resets to that initial item whenever `active` turns false, so the menu reopens where
  it started, as `closeMenu` does in the RN products.
- An arrow key moves the tab stop and focuses the item's host through `asFocusTarget`. It calls
  `preventDefault` only for keys it handles. Enter, Space, Escape, and Tab pass through: activation
  stays with the product's `Pressable` or button, and Escape reaches the `useOverlayA11y` container
  handler.
- A disabled or removed active item: the tab stop stays at the same index while that index holds
  an enabled item, and otherwise falls back to the initial item. Item refs are cached per index, so
  React calls a ref callback only on mount, unmount, or reorder, and an unmounted item's node is
  cleared. If the unmounting item held DOM focus and focus fell to `document.body`, an effect
  focuses the new active item. Focus elsewhere is left alone.
- A nested keyboard target: when the key event's `target` is an object other than the item's own
  host, such as a text field inside the item, the hook ignores the key and does not call
  `preventDefault`. React Native events carry a numeric target or none, so the check only applies
  to hosts that expose DOM nodes.
- No enabled items: `activeIndex` is null, every item gets `tabIndex: -1`, and arrow keys do
  nothing.
- There is no `Platform.OS` gate. The RN products return early off web, but a key event only
  arrives where the host delivers one, and focusing a native host without `focus()` is a no-op.

### Deviations from the study

- The study sketched `initialFocusRef` as `undefined` when no item is enabled, with the product
  passing its close control as the fallback. `useFocusTrap` lists the ref in its effect
  dependencies, so a ref that changes identity while the menu is open reruns the entry effect. The
  rerun records the menu item that holds focus as the element to restore, and the trigger is lost
  on close. The hook therefore returns one stable ref whose `current` is null with no enabled item.
  `useFocusTrap` already falls back to the first focusable element in the container, which is the
  close control in every surveyed menu that has one. A test disables every item while the menu is
  open and still restores the trigger. The three RN products switch between `firstItemRef` and
  `closeRef` and have this defect today.
- The study said the hook "resets when an open menu receives a new option list". Products build
  the item array during render, so its identity changes on unrelated parent renders, and a reset
  would throw away arrow-key movement. The hook reconciles by index instead, as described above.
- `RovingMenuItemProps.ref` and `initialFocusRef` use `object` instead of `View`, so the module
  stays importable from the web entry. `useOverlayA11y` widens `initialFocusRef` to `HostRef` to
  accept it; a `View` ref is still assignable to both.
- `RovingMenuKeyEvent` is a new type rather than the radio group's `RovingKeyEvent`, because it
  adds `target` and the radio group module imports React Native types.

## Tests

`src/use-roving-menu.test.ts` runs under jsdom. The package has no React Native Web dependency; it
simulates React Native Web the way the radio group and overlay tests do, with `Platform.OS` mocked
to `web` and DOM hosts attached to the refs.

- Pure vectors: skip disabled items in both directions, wrap, Home and End with disabled edges, a
  disabled current item, a removed current item, `-1`, one enabled item, no enabled items, empty
  list, ignored keys, and Redemut's disabled-first and selected vector.
- Hook state: selected item start, disabled selected item, disabled-only menu, Enter, Space,
  Escape, and Tab left alone, movement before hosts mount, reset after close, fallback when the
  active item is disabled or removed, and a stable `initialFocusRef`.
- Rendered web menu with `useOverlayA11y` and real `keydown` events: entry on the selected item,
  roving with wrap, Home, End, and Space; Hebkit's disabled middle item; React Native style events
  on host refs; disabled-only menu focusing the close control; Escape from an item closing without
  an action and restoring the trigger; reopening at the initial item; removal of the focused item,
  the focused last item, and an unfocused item; all items disabled while open still restoring the
  trigger; a nested field keeping its keys; a rejected action that reopens the menu; and unmounting
  the open menu as a route change does, including a stale handler afterwards.
- Native with `useOverlayA11y`: accessibility focus enters the container, back (the product sets
  `active` false from `onRequestClose`) restores the trigger, and the tab stop resets.

Temporarily disabling focus recovery and the nested-target check fails three of these cases.

After `pnpm build`, Node imports `dist/web.js` and finds both exports, and neither
`dist/use-roving-menu.js` nor its `.d.ts` mentions `react-native`.

`make ts-browser-test` passes but runs only the Dexie package in Playwright. No real-browser menu
test exists; the jsdom cases above use real DOM focus and events. A browser case belongs in the
adopting product's Playwright suite.

## Supported runtimes

React 19.2 with React DOM, React Native 0.86, and React Native Web through the root entry. Plain
React web apps through `/web`. On iOS and Android, where `Pressable` receives no key events from a
screen reader, the hook contributes only `tabIndex` and refs, and accessibility focus stays with
`useOverlayA11y`. No VoiceOver or TalkBack run was made for this change.

## Failure behavior

The hook never invokes an item action, so an action that throws or rejects cannot corrupt its state.
The product decides whether a failure reopens the menu, moves focus, or announces an error; the
test reopens the menu and the tab stop starts at the initial item. A focus call on a host without
`focus()` is skipped. A handler that runs after unmount finds no host and does nothing.

## Privacy boundary

The hook holds indexes and host references in memory. It reads no labels and logs nothing.

## Breaks

None. The only change to an existing export widens `OverlayA11yOptions.initialFocusRef`.

## Product adoption follow-ups (step 4, deferred)

- Eigenruhe `mobile/src/components/context-menu.tsx`: replace `firstEnabledIndex`,
  `selectedIndex`, `initialItemIndex`, `firstItemRef`, `itemRefs`, `activeItemIndex`, the
  `setActiveItemIndex` call in `closeMenu`, and `moveItemFocus` with `useRovingMenu`; pass
  `menu.initialFocusRef` to `useOverlayA11y` and spread `menu.itemProps(index)` on each item.
- Tiefgang `mobile/src/components/context-menu.tsx`: the same removal; keep its `deferFocus`.
- Hebkit `mobile/src/components/context-menu.tsx`: run `context-menu.test.tsx` against the hook,
  then make the same removal.
- Redemut `packages/ui/src/context-menu.tsx`: the menu is DOM-only and has no overlay, so either
  move item keys onto `useRovingMenu` from `@baukit/a11y-core/web` or call `nextEnabledMenuIndex`
  from the container handler. Keep its outside-pointer dismissal. Rerun
  `packages/ui/test/components.test.tsx`.
- Solo Leveling System `mobile/src/components/context-menu.tsx`: delete the hand-rolled trap in the
  visibility effect and the manual restoration, adopt `useOverlayA11y` and `useRovingMenu`, and
  move `mobile/src/components/confirmation-dialog.tsx` onto `useOverlayA11y` as well.

## Product defects found

- Eigenruhe, Hebkit, Tiefgang: `initialFocusRef` switches between `firstItemRef` and `closeRef`
  when the enabled items change while open, which makes the web trap forget the trigger.
- Eigenruhe, Hebkit, Tiefgang, Redemut: `Math.max(0, position) + 1` skips the first enabled item
  when focus is on a disabled item or, in Redemut, on menu content that is not an item; Arrow Up
  from there lands on the second-to-last item.
- Eigenruhe, Hebkit, Tiefgang: `activeItemIndex` is initialized once from the first render and
  reset only by `closeMenu`, so items that change while the menu is closed can leave the tab stop
  on a disabled or missing item.
- Solo Leveling System: no arrow-key movement, and neither the menu nor the confirmation dialog uses
  `useOverlayA11y`, so the background gets no `inert` on web and no hiding props on native. The item
  action runs synchronously right after `onClose`.

## Follow-up 0.5.1 (2026-09-29)

Source revisions: Baukit baseline 28db260, Redemut 1d40e90. This note is the a11y-core note in
the fix plan, so the reduced-motion follow-up lands here.

### Product evidence

`src/use-reduced-motion.ts` imports `AccessibilityInfo` from `react-native`, and `src/web.ts` did
not export `useReducedMotion`. Redemut's web app therefore keeps `web/src/reduced-motion.ts`, a
`matchMedia` hook used by `web/src/dialog-screens.tsx:53` and
`web/src/celebrations/use-one-shot.ts:3`. Importing the root entry from a plain React web build
would pull React Native into the bundle.

### Decision

`@baukit/a11y-core/web` exports `useReducedMotion`, `useReducedMotionPreference`, and the
`ReducedMotionPreference` type from `src/use-reduced-motion-web.ts`, which imports only React. The
hooks read `(prefers-reduced-motion: reduce)` through `useSyncExternalStore` and follow changes to
the query. The server snapshot is `{ reduceMotion: false, resolved: false }`, so server rendering
and hydration match. A client-only render resolves on its first render, and a hydrated one right
after hydration. The native root hook keeps
its asynchronous `AccessibilityInfo` query; the web hooks have the same names and return shape, so
shared component code reads the same fields on both entries. The query string lives in the web
file and the root hook imports it from there, so the two entries cannot drift.

### Gates

TypeScript workspace, run from `typescript/`: `build`, `format:check`, `lint`, `test`, and `check`
all passed. The new test mocks `react-native` to throw on import, so it fails if the web entry
ever reaches React Native. It covers the first client render, a change event and unsubscribe on
unmount, a host without `matchMedia`, a stable return object, and a hydrated render that starts
unresolved and then resolves.

### Breaks

None. The root entry keeps its exports and behavior.

### Product adoption

Redemut: delete `web/src/reduced-motion.ts` and import `useReducedMotion` from
`@baukit/a11y-core/web` in `web/src/dialog-screens.tsx` and `web/src/celebrations/use-one-shot.ts`.

## Follow-up 0.5.2 (2026-09-30)

### Product evidence

Schlauzug at `01124c8` keeps `web/src/live-announcer.tsx` (`useLiveAnnouncer` and `LiveRegion`)
in six screens (`run-screen.tsx`, `play-home.tsx`, `room-screen.tsx`, `about-screen.tsx`,
`run-setup-screen.tsx`, `profile-screen.tsx`). `announce` was exported only from the
`@baukit/a11y-core` root, and `src/announce.ts:1` imports `react-native`, so the plain React web
app could not use it. The hook also drops routine game messages through
`isRoutineGameAnnouncement`, which is product policy.

### Decision

The live-region code moves from `src/announce.ts` to `src/announce-web.ts`, which imports nothing.
`@baukit/a11y-core/web` exports `announce(message, { assertive, liveRegionId })`,
`AnnounceOptions`, and `DEFAULT_LIVE_REGION_ID` from it. The root `announce` keeps its name and
options: it calls `announceForAccessibility` on iOS and Android and delegates to the web module on
React Native Web, so the two entries cannot drift. The region is the same as before: a visually
hidden element with `aria-atomic="true"`, `aria-live` polite or assertive, and `role` status or
alert, created on first use or adopted when the product rendered one under the id. The text is
cleared and a reflow forced before each message, so repeating a message is spoken again. Blank
messages are dropped. Filtering routine messages stays in the product.

### Gates

- `@baukit/a11y-core` test: the web tests moved to `announce-web.test.ts`, which mocks
  `react-native` to throw on import. `announce.test.ts` keeps the native cases and adds one for
  React Native Web. `web.test.ts` pins the web entry's export list with `announce` and
  `DEFAULT_LIVE_REGION_ID`.
- Whole TypeScript workspace `build`, `format:check`, `lint`, `test`, and `check`: pass.
- Generated fixture `--backend --mobile --web`: mobile `tsc --noEmit`, lint, and
  `test:coverage` (35 tests, including the template's root `announce` test) and web build, lint,
  test, and `test:coverage` pass.

### Breaks

None. The root entry keeps its exports and behavior.

### Product adoption

Schlauzug: delete `web/src/live-announcer.tsx` apart from `isRoutineGameAnnouncement` (move it
next to the game code), remove `<LiveRegion>` and `useLiveAnnouncer` from the six screens, and call
`announce(message, { assertive })` from `@baukit/a11y-core/web` after the routine-message check.
Its `.visually-hidden` class is no longer needed for the region.
