---
name: baukit-accessibility
description: Implement or review accessible Baukit web and React Native screens using the platform contract, generated lint and axe gates, native semantic test seams, and recorded VoiceOver/TalkBack release evidence. Use for overlays, focus behavior, announcements, reduced motion, forms, touch targets, data graphics, accessibility regressions, or native release-readiness reviews.
---

# Implement and review accessible screens

Read `<baukit-repo>/docs/platform/accessibility-contract.md` before changing product code. Keep components, navigation, translated copy, visual design, chart summaries, and exact call sites product-owned.

## Inventory the user path

1. List every state on the changed path: loading, empty, error, invalid, ready, overlay open/closed, submit failure/success, and reduced motion.
2. Identify the intended name, role, value/state, heading, reading order, target size, focus destination, dismissal, announcement, and non-visual alternative for each state.
3. Identify the supported iOS, Android, browser, locale, text-size, and input combinations. Treat keyboard, touch, switch/screen-reader traversal, and deep-link entry as distinct paths.

## Implement the contract

Use this checklist:

- Give controls and graphics meaningful names, roles, state/value, reading order, and non-color cues. Keep real heading semantics independent of typography.
- Meet the product target minimum: web 44 by 44 CSS pixels; native 44 by 44 points on iOS and 48 by 48 dp on Android, with separable hit areas.
- Link form labels, help, required/invalid state, and errors. Announce the result and focus the first invalid field without erasing input.
- Use `useReducedMotionPreference` when startup motion depends on the preference. Wait for `resolved` before starting non-essential motion because the native query is asynchronous.
- Announce important asynchronous outcomes once, using localized, task-specific copy.
- Give every data graphic a structured text/table alternative with relevant values, units, range, trend, and exceptions.

For web route focus, create one `createRouteFocusController` before navigation starts. It preserves
the real initiating control when an inert outgoing scene makes WebKit blur focus to `body`. Pass a
getter for the destination heading or primary target, and run the returned cleanup when the route
becomes inactive. Do not focus inert or `aria-hidden="true"` targets.

For a web modal, provide initial focus, Tab containment, Escape behavior, inert background content, and focus restoration.

For a native overlay:

- Require stable trigger and overlay container/heading refs from the caller. Never infer previous native focus in a generic hook.
- Request entry focus from a layout/presentation callback, not during render. Test that the focus call happens only after layout.
- Put `accessibilityViewIsModal` on iOS overlay content. Put iOS `accessibilityElementsHidden` and Android `importantForAccessibility="no-hide-descendants"` on the app content _behind_ the overlay.
- Restore focus to the supplied trigger when it still exists; otherwise use an explicit stable fallback.

Use generated helpers as reference seams, not as a UI framework. Adapt them when product navigation or overlay ownership differs.

## Add evidence

Add focused tests for every changed behavior. On native, switch `Platform.OS` and assert both iOS and Android props/events; inject or spy on announcement, reduced-motion, native-handle, and focus seams. On web, extend the jsdom axe scan and interaction tests for the changed state.

From a generated product root, run the applicable fast gates:

```sh
corepack pnpm --dir mobile typecheck
corepack pnpm --dir mobile lint
corepack pnpm --dir mobile test

corepack pnpm --dir web build
corepack pnpm --dir web lint
corepack pnpm --dir web test
```

Keep lint at zero warnings. The generated web suite fails on serious/critical axe violations; broaden it to relevant dialogs and route states. Fix violations rather than excluding rules unless the exclusion documents a tool limitation and has equivalent evidence.

## Complete the device gate

Run both platform passes from the contract's "VoiceOver and TalkBack release protocol (version 1)" on a release binary. Record build, hardware, OS, screen-reader version, locale, text size, reduced-motion setting, critical paths, findings, and `PASS | FAIL | BLOCKED` separately for iOS and Android.

Do not claim the native baseline from prop tests, lint, axe, simulator snapshots, or a single platform. Automated evidence cannot judge spoken copy, timing, traversal, gesture operation, focus restoration, or the usefulness of a graphic summary.
