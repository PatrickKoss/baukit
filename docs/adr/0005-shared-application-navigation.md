# ADR 0005: Shared application navigation

## Status

Accepted, 2026-10-03.

Supersedes the navigation ownership decision in [ADR 0001](0001-product-experience-package-boundaries.md),
the [next improvements plan](../next-improvements-plan.md) at its product-owned navigation exclusion,
the [Expo UI and accessibility study](../studies/31-expo-ui-and-headless-accessibility.md), and the introduction to
the [accessibility contract](../platform/accessibility-contract.md). Their other package boundaries and
accessibility requirements still apply.

## Context

The user asked all eight products to adopt one collapsible navigation pattern.
Hebkit, Eigenruhe, Tiefgang and Solo Leveling use Expo Router and React Native
Web. Leitbild, Redemut, Runtime Analyzer and Schlauzug also have DOM web apps.
Products use Ionicons, Lucide, inline SVG and glyphs. Their routers include
Expo Router, TanStack Router and pushState. Their styling includes StyleSheet,
plain CSS and Tailwind.

Hebkit supplies the compact bar and active-tab cycle. Schlauzug and Redemut
supply the placement of sub-items under their parent. Detached section blocks
and profile entries in the top-right or a More menu do not meet the requested
pattern. The existing headless hooks and layout helpers can support it without
moving routing or product data into Baukit.

## Decision

Publish `@baukit/navigation` at the current Baukit version. The root entry is a
pure TypeScript model. The web entry renders anchors and disclosure buttons and
has a token-based stylesheet. The native entry renders React Native components
and accepts a product-built `NavigationTheme`. React Native and React DOM are
optional peers. No entry imports a router or an icon library.

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

## Consequences

This is a narrow rendered package for navigation. It does not authorize a
shared settings UI, general Expo component library or product layout framework.
Products still own routes, labels, icons, copy, semantic tokens, menu actions,
authentication, persistence and screen composition.

The web and native entries require separate component tests. The DOM entry has
Playwright and axe checks at the breakpoint boundaries. Generated web and
mobile fixtures exercise router wiring and dependency injection. Product
adoption must still test actual route trees, large text, VoiceOver and TalkBack.
