# 31. Expo UI and headless accessibility behavior

## Source product files

Inspected revisions: Tiefgang `861cf0a994d5e63ec245e645023c80575759c191`, Eigenruhe `36b468d015f4aebd83a11bd662c7ff82124711fb`, and Redemut `b4e8a9872595260d3f26af7d8d085aac98485e51`.

- `/home/patrick/projects/tiefgang/mobile/src/components/ui.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/components/context-menu.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/components/detail-route-state.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/components/bar-chart.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/features/settings/segmented-tabs.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/navigation/tab-content-inset.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/navigation/context-menu.test.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/route-state.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/components/form-input.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/bottom-sheet.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/context-menu.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/segmented-tabs.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/slider.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/toast.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/accessible-modal.web.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/components/charts/chart-table-alternative.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/navigation/tab-content-inset.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/route-state.ts`
- `/home/patrick/projects/redemut/packages/ui/src/context-menu.tsx`
- `/home/patrick/projects/redemut/packages/ui/test/components.test.tsx`
- `/home/patrick/projects/redemut/mobile/src/mobile-shell.tsx`
- `/home/patrick/projects/redemut/mobile/src/settings-screen.tsx`
- `/home/patrick/projects/redemut/mobile/src/route-state.ts`
- `/home/patrick/projects/redemut/web/src/App.tsx`

Fitness Tracker is not checked out on this machine. No source path or current API was available for inspection.

## Observed failure or repeated glue

Tiefgang, Eigenruhe, and Redemut each implement enabled-item filtering, initial menu focus, Arrow, Home, and End movement, disabled-item skipping, and trigger restoration. The rendered menus differ. Tiefgang and Eigenruhe also repeat route-state derivation almost byte for byte, but that code is navigation state rather than accessibility behavior. No checked-out product has a completed VoiceOver or TalkBack record, a disabled-only menu test, or a tested modal stack.

## Baukit owner

`@baukit/a11y-core` should own `useRovingMenu` and the pure enabled-index helper. Existing `useOverlayA11y` continues to own overlay focus, inert background, Escape, and restoration. `docs/platform/navigation-contract.md` remains the owner of route recovery. No `@baukit/ui-expo` package is proposed by this study.

## Public types and errors

The sketch names `RovingMenuOption`, `RovingMenuOptions`, `RovingMenuItemProps`, `RovingMenuResult`, `nextEnabledMenuIndex`, and `useRovingMenu`. It adds no public error type. Empty, disabled-only, stale-index, and unmounted-ref cases return neutral state and do not throw. Action failures never enter the hook because the product invokes actions.

## Product-owned inputs

Products supply visibility, option order, disabled and selected state, stable refs, close fallback, action callbacks, copy, icons, tones, placement, layout, tokens, routes, analytics, and failure recovery. They also own field validation, slider ranges, toast policy, chart summaries, and modal-stack policy.

## Concurrency, failure, privacy, and cleanup cases

Tests must cover option changes while open, rapid keys before React commits, one item, no items, every item disabled, a selected disabled item, removed active items, unmounted refs, repeated open and close, top-only Escape in product stacks, and trigger removal. The hook stores no labels or product values in errors or logs. Unmount clears refs and leaves no document listener. Product action failures must not strand focus or expose error text through library diagnostics.

## Supported runtimes

The root export supports React Native on the iOS and Android versions supported by the consuming Expo SDK. The `/web` export supports the package's current ES2022 React web and React Native Web targets without importing React Native at runtime or in its types. Products still need real VoiceOver, TalkBack, Chromium, and WebKit evidence.

## Product adoption change

Tiefgang can delete `activeItemIndex`, `itemRefs`, enabled-index calculation, and `moveItemFocus` from `mobile/src/components/context-menu.tsx`. Eigenruhe can delete the same code from its corresponding file. Redemut can delete the DOM query and `moveFocus` implementation from `packages/ui/src/context-menu.tsx`. Their rendered menus remain local. No Fitness Tracker deletion can be named until its repository is available.

## Throwaway experiments

None. The study used source inspection only.
