# 31. Expo UI and headless accessibility behavior

## Question and scope

Should Baukit add headless accessibility behavior or a rendered Expo component package for the labeled fields, overlays, switches, segmented choices, route states, chart alternatives, context menus, safe-area helpers, sliders, toasts, and modal stacks now present in Tiefgang, Eigenruhe, and Redemut? This study compares props, state, and tests against `@baukit/a11y-core`, ADR 0001, the accessibility contract, and the navigation contract. Fitness Tracker is not checked out on this machine, so its settled Wave 3 API could not be inspected. Product copy, visual tokens, route trees, graphic summaries, and screen composition remain outside Baukit.

## Evidence table

| Product or Baukit area | File | What it does | What varies between products and boundary finding |
| --- | --- | --- | --- |
| Baukit headless behavior | `typescript/packages/a11y-core/README.md` and `typescript/packages/a11y-core/src/index.ts` | Exports overlay focus and inert behavior, announcements, reduced-motion state, roving radio groups, form Enter movement, route focus, and single-flight actions. It renders no components. | This is the accepted home when interaction state repeats. It has no menu navigation, field association, toast queue, slider state, or modal-stack coordinator. |
| Baukit package boundary | `docs/adr/0001-product-experience-package-boundaries.md` | Defers `@baukit/ui-expo` until settled component APIs have a second consumer and can accept caller tokens, copy, and layout. | Tiefgang and Eigenruhe now provide two implementations, but most rendered APIs still differ. Fitness Tracker cannot be checked for compatibility here. |
| Baukit contracts | `docs/platform/accessibility-contract.md` and `docs/platform/navigation-contract.md` | Define field semantics, graphics alternatives, overlay behavior, device evidence, route focus, and back-or-replace recovery. | They intentionally leave components, routes, copy, and graphics local. The navigation contract does not include the repeated route-state derivation function. |
| Labeled fields | `/home/patrick/projects/tiefgang/mobile/src/components/ui.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/components/form-input.tsx`, and Redemut fields in `/home/patrick/projects/redemut/mobile/src/learning-card.tsx` | Tiefgang's `FormInput` creates label, help, and error IDs and sets invalid state. Eigenruhe renders label, input, and live error. Redemut labels individual product fields directly. | The visible label and error layout repeats, but the props do not. Only Tiefgang proves `aria-labelledby`, `aria-describedby`, and invalid state. This needs contract conformance, not shared rendering yet. |
| Overlays | `/home/patrick/projects/tiefgang/mobile/src/components/ui.tsx`, `/home/patrick/projects/tiefgang/mobile/src/components/context-menu.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/components/bottom-sheet.tsx`, and `/home/patrick/projects/eigenruhe/mobile/src/components/context-menu.tsx` | Both products combine `useOverlayA11y` with a modal, heading, dismiss control, background, and reduced-motion animation choice. | Focus, inert background, Escape, and restoration already belong to `a11y-core`. Rendered APIs differ between free-form children, action arrays, form variants, subtitles, placement, and icons. No new overlay component is justified. |
| Switches | `/home/patrick/projects/tiefgang/mobile/src/components/ui.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/features/preferences/preferences-screen.tsx`, and `/home/patrick/projects/redemut/mobile/src/settings-screen.tsx` | Tiefgang renders a custom Pressable switch. Eigenruhe and Redemut often use the native `Switch`, while some Eigenruhe player controls use a Pressable with switch semantics. | Checked and disabled semantics repeat. Rendered output and label placement do not. A boolean value has no reusable state machine, so this stays local under the accessibility contract. |
| Segmented and roving choices | `/home/patrick/projects/tiefgang/mobile/src/features/settings/segmented-tabs.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/components/segmented-tabs.tsx`, and `/home/patrick/projects/redemut/mobile/src/guided-screen.tsx` | Tiefgang and Eigenruhe use `useRovingRadioGroup`; Redemut renders product-specific radio choices. | Arrow, Home, End, wrapping, selection, and one tab stop are already shared. Eigenruhe adds scrolling and short labels. Redemut choices include learning feedback. Only interaction state is common, and Baukit already owns it. |
| Route-state views | `/home/patrick/projects/tiefgang/mobile/src/route-state.ts`, `/home/patrick/projects/eigenruhe/mobile/src/route-state.ts`, `/home/patrick/projects/redemut/mobile/src/route-state.ts`, and `/home/patrick/projects/tiefgang/mobile/src/components/detail-route-state.tsx` | All three derive mutually exclusive loading, invalid, not-found, error, and ready states. Tiefgang and Eigenruhe source is byte-identical; Redemut differs only in formatting. | Pure state repeats, but it is navigation and data-loading behavior rather than accessibility behavior. Rendered titles, recovery destinations, and error treatment vary. Keep the function local until the navigation contract takes an implementation export. |
| Chart alternatives | `/home/patrick/projects/tiefgang/mobile/src/components/bar-chart.tsx`, `/home/patrick/projects/tiefgang/mobile/src/components/ui.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/components/charts/chart-frame.tsx`, and `/home/patrick/projects/eigenruhe/mobile/src/components/charts/chart-table-alternative.tsx` | Tiefgang gives charts a caller summary and switches caller-rendered chart or table. Eigenruhe owns row types, structured summaries, table rendering, and dense chart navigation. Redemut web charts expose product summaries directly in `/home/patrick/projects/redemut/web/src/stats-screens.tsx`. | The requirement repeats, not one data model. Values, grouping, units, trends, and exceptions are product meaning. Keep rendering local and use contract tests. |
| Context menus | `/home/patrick/projects/tiefgang/mobile/src/components/context-menu.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/components/context-menu.tsx`, and `/home/patrick/projects/redemut/packages/ui/src/context-menu.tsx` | Each finds enabled items, chooses initial focus, skips disabled items for Arrow keys, supports Home and End, closes on Escape, and restores trigger focus. Tiefgang and Eigenruhe also use `useOverlayA11y`. | The menu item state and keyboard code repeats across React Native Web and DOM. This is a clear `a11y-core` candidate. Rendered menus still differ in dialog use, header shape, descriptions, icons, placement, and theming, so a later `ui-expo` component would need too many choices today. |
| Safe-area helpers | `/home/patrick/projects/tiefgang/mobile/src/navigation/tab-content-inset.tsx`, `/home/patrick/projects/eigenruhe/mobile/src/navigation/tab-content-inset.tsx`, and `/home/patrick/projects/redemut/mobile/src/mobile-shell.tsx` | Tiefgang and Eigenruhe provide almost the same bottom-tab inset context. Redemut reads safe-area context directly for its menu and tab bar. | Inset arithmetic repeats, but it depends on each navigation layout and provider placement. Numeric layout helpers belong in `ui-tokens`; provider composition stays local. No rendered component qualifies. |
| Sliders | `/home/patrick/projects/eigenruhe/mobile/src/components/slider.tsx` and `/home/patrick/projects/eigenruhe/mobile/src/components/slider.test.tsx` | Implements pointer, drag, native accessibility actions, web keys, step rounding, value text, and a 44-point target. | No second slider exists in the checked-out products. Keep it in Eigenruhe. A future comparison must settle orientation, reversed ranges, disabled state, RTL keys, and continuous values. |
| Toasts | `/home/patrick/projects/eigenruhe/mobile/src/components/toast.tsx` and `/home/patrick/projects/redemut/web/src/App.tsx` | Eigenruhe queues three timed actionable toasts and disables their motion when unresolved or reduced. Redemut renders separate signed-in and offline live regions with local dismissal. | Announcement and dismissal are common, but queueing, timeout, action lifetime, and stacking do not match. The only shared operation is already `announce`. Keep rendering and queue policy local. |
| Modal stacking | `/home/patrick/projects/eigenruhe/mobile/src/components/accessible-modal.web.tsx` | Keeps a global host stack, makes only the top modal active, and restores original `aria-hidden` and `inert` values. | Tiefgang portals a modal but has no stack coordinator. Redemut composes dialogs through its web focus helpers. One implementation and no stack test are insufficient for extraction. |
| Fitness Tracker | Not available locally | ADR 0001 names it as the original Expo source and says its Wave 3 call sites were changing. | No current props, state, or tests could be compared. This blocks a claim that a new rendered package can replace its components. |

No requested family currently proves a coherent first `@baukit/ui-expo` release. Context menus come closest, but their shared part is the enabled-item focus state. The visual and composition APIs are still different.

## Candidate interface or contract sketch

The evidence supports one additive headless export. It should complement `useOverlayA11y`, not own a modal or invoke an item action.

```ts
interface RovingMenuOption {
  readonly disabled?: boolean;
  readonly selected?: boolean;
}

interface RovingMenuOptions {
  readonly active: boolean;
  readonly options: readonly RovingMenuOption[];
}

interface RovingMenuItemProps {
  readonly onKeyDown: (event: RovingKeyEvent) => void;
  readonly ref: (node: View | null) => void;
  readonly tabIndex: 0 | -1;
}

interface RovingMenuResult {
  readonly activeIndex: number | null;
  readonly initialFocusRef: RefObject<View | null> | undefined;
  readonly itemProps: (index: number) => RovingMenuItemProps;
}

function nextEnabledMenuIndex(
  key: string,
  currentIndex: number,
  options: readonly RovingMenuOption[],
): number | null;

function useRovingMenu(options: RovingMenuOptions): RovingMenuResult;
```

The hook chooses a selected enabled item first, otherwise the first enabled item. It resets when an open menu receives a new option list. Arrow keys wrap among enabled items; Home and End select the first and last enabled items. With no enabled items it returns `activeIndex: null`, no initial item ref, and every item has `tabIndex: -1`. The product passes a close control to `useOverlayA11y` as the initial focus fallback. Escape remains in `useOverlayA11y`. Action order, async failure, dismissal, selection meaning, and copy remain with the product.

The existing navigation contract should keep route-state recovery as a documented product pattern for now. If Baukit later publishes `deriveDetailRouteState`, it belongs beside `backOrReplace`, not in `a11y-core` or `ui-expo`.

## Required-case coverage

| Required case | Coverage today | Required package behavior or missing proof |
| --- | --- | --- |
| iOS screen reader | Tiefgang's `mobile/src/navigation/context-menu.test.tsx` asserts modal state, hidden background props, and focus only after layout for `ios`. Tiefgang and Redemut announcement tests exercise `announceForAccessibility`. | No checked-out product has a completed VoiceOver release record. Run the accessibility contract's device protocol against every adopted overlay and menu. |
| Android screen reader | The same Tiefgang context-menu test asserts `importantForAccessibility="no-hide-descendants"` and delayed focus for `android`. | No completed TalkBack record was found. Prop tests do not prove traversal, spoken selection, or back behavior. |
| Web keyboard use | Tiefgang Playwright tests cover sheet and menu entry, Escape, and restoration. Eigenruhe tests roving radios and slider keys. Redemut's `packages/ui/test/components.test.tsx` covers menu Arrow keys, Home, End, Escape, and disabled-item skipping. | Add DOM tests for the new hook, including dynamic options, wrapping, selected-disabled input, one item, no items, and every item disabled. |
| Focus restoration | `useOverlayA11y` tests web and native restoration. Tiefgang verifies it in a browser. Redemut mobile restores profile-menu focus explicitly. | Adoption must remove product menu focus code without weakening a stable trigger or fallback. Nested overlays need an ordered restoration test. |
| Escape and back behavior | Tiefgang and Redemut browser suites close overlays with Escape. All three mobile apps test the same back-or-replace branches. Native modals wire `onRequestClose`. | Test Android hardware back for each adopted overlay. Escape should close only the top overlay and must not invoke an item. |
| Reduced motion | Tiefgang and Eigenruhe use `useReducedMotionPreference` for sheets and menus. Eigenruhe tests static celebrations and sliders; Tiefgang tests a non-pulsing status dot. | No package change is needed. Product browser and native tests must prove the no-motion path before a modal appears, including unresolved native preference state. |
| Large text | Eigenruhe segmented tabs support scrollable full labels and compact labels. Some layout tests set `fontScale`, but only to `1`. | No product proves large accessibility text. Device tests must cover clipped field errors, menu items, sheet headings, slider values, toast actions, and chart tables at the supported maximum. |
| High contrast | Tiefgang and Eigenruhe run numeric contrast tests over semantic tokens. Redemut's browser axe suite includes contrast checks. | There is no forced-colors or native high-contrast record. Keep color pairs in product token tests and add platform checks where the OS exposes a setting. |
| Disabled-only menus | All three menu implementations skip disabled items. Eigenruhe and Tiefgang fall back to the close ref when no enabled item exists. | No current test opens a menu whose items are all disabled. The new hook must return no item tab stop and let the overlay focus the close control. |
| Action failure | Redemut mobile catches profile sign-out failure and renders an alert. Generic context-menu actions in all three implementations close before running and do not define async failure. | The hook must never invoke actions. Products must decide whether failure reopens the menu, moves focus, shows a toast, or announces an error, then test that choice. |
| Routing | All three mobile apps have `DetailRouteState` and back-or-replace unit tests. Tiefgang and Redemut run browser route-state recovery tests; Eigenruhe tests protected deep links and unknown actions. | Keep route identifiers and fallback destinations local. Add route-state cases to the navigation contract only after a neutral error representation is settled. |
| Stacked overlays | Eigenruhe's web modal tracks a host stack and activates only the last host. | No test covers two open modals, top-only Escape, background restoration after closing one, or trigger restoration order. Keep stacking local until a second product and these tests exist. |

The new hook also needs root-entry and `/web` packed-artifact tests. Its web entry must remain free of a runtime or type dependency on React Native, following the current `a11y-core` entry-point rule.

## Decision

Decision: implement one additive `useRovingMenu` export in `@baukit/a11y-core`, with a pure `nextEnabledMenuIndex` helper and conformance tests for the cases above. Do not create `@baukit/ui-expo` in this item. The repeated menu code is interaction state, while every rendered family still has incompatible props or only one proven implementation. The smallest next step is to write the hook tests against DOM hosts and mocked native refs, then replace the enabled-item index, ref, and key-handling code in Tiefgang, Eigenruhe, and Redemut. Fitness Tracker can be checked during adoption; it is not needed to prove this headless menu behavior, but it remains required evidence before opening a rendered Expo package.

## What stays product-owned

- All components, styles, semantic tokens, icons, copy, headings, descriptions, action arrays, placement, breakpoints, safe-area provider placement, and animation choice.
- Menu visibility, trigger and fallback refs, dismissal policy, action invocation, async failure recovery, destructive confirmation, analytics, and routing.
- Field schemas, validation, help and error copy, focus-to-first-error behavior, and whether errors appear inline or in a summary.
- Switch meaning, segmented-choice labels and layout, slider ranges and units, toast limits and duration, chart data and summaries, and route-state copy.
- Modal-stack ownership until a second implementation and top-only dismissal tests exist.
- VoiceOver and TalkBack release records, large-text checks, forced-color or high-contrast checks, and browser QA configuration for each product's actual routes and overlays.
