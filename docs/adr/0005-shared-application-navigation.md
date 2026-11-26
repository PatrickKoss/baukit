# ADR 0005: Products share one application navigation package

## Status
Accepted, 2026-10-03.

## Context

Every product needed one collapsible navigation pattern. Some products use Expo Router and React
Native Web, others also have DOM web apps. They use different icon sets, routers (Expo Router,
TanStack Router, plain pushState) and styling (StyleSheet, plain CSS, Tailwind). Detached section
blocks and profile entries hidden in a More menu did not meet the pattern.

One product supplied the compact bar and the active-tab cycle, and two others supplied the placement
of sub-items under their parent. The headless hooks and layout helpers baukit already had could
support the pattern without moving routing or product data into baukit.

## Decision

`@baukit/navigation` ships on the release train. The root entry is a
pure TypeScript model. The web entry renders anchors and disclosure buttons and
has a token-based stylesheet. The native entry renders React Native components
and accepts a product-built `NavigationTheme`. The package has no runtime dependencies. Its peers
are `react`, `@baukit/a11y-core` and `@baukit/ui-tokens`, which every product installs, and
`react-dom` and `react-native`, which are optional so each entry needs only its own renderer. No
entry imports a router or an icon library.

Use a bottom bar below 1024 pixels and a rail at or above 1024 pixels. Rail rows
have an icon left of the label. Compact rows have an icon above the label.
A rail can collapse to icons, with accessible labels and web titles. A
section disclosure owns an indented group immediately after its button. The
active section starts open. Clicking a collapsed disclosure expands the rail
and opens the group. Active leaves have a muted accent background with accent
text. The parent of an active child has accent text and icon without a fill.

Keep profile last, at the rail bottom or the bar end. The product supplies its
avatar or initials and either a destination or menu entries. Compact inactive
tabs go to the section root; active tabs cycle through the section's children.
A section picker provides direct access to those children.

Reuse `a11y-core` for focus movement, reduced-motion state, inert synchronization
and announcements. Products own route-heading focus and native screen-reader
release evidence. Preserve the navigation recipe's links, focus, targets,
safe areas and browser checks.

## Enforced by

- `typescript/packages/navigation/src/index.test.ts`: "uses the token layout helper at the rail
  boundary" pins the 1024 pixel switch, and "rejects seven main items" and "rejects duplicate ids
  across parents, children and profile entries" pin the model rules.
- `typescript/packages/navigation/src/web.browser.test.tsx` and `native.browser.test.tsx` render
  both entries in a browser.
- Nothing checks that no entry imports a router or an icon library.

## Consequences

This is a narrow rendered package for navigation. It does not authorize a
shared settings UI, general Expo component library or product layout framework.
Products still own routes, labels, icons, copy, semantic tokens, menu actions,
authentication, persistence and screen composition.

The web and native entries require separate component tests. The DOM entry has
Playwright and axe checks at the breakpoint boundaries. Generated web and
mobile fixtures exercise router wiring and dependency injection. Product
adoption must still test actual route trees, large text, VoiceOver and TalkBack.

## History

- 2026-10-03, commit `0157313f`: accepted. It replaced the navigation ownership part of
  [ADR 0001](0001-product-experience-package-boundaries.md) and the introduction of the
  [accessibility contract](../platform/accessibility-contract.md). Their other boundaries and
  accessibility rules still apply.
- 2026-11-27: added the required peers and removed product names.
