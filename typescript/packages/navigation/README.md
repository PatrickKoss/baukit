# @baukit/navigation

Collapsible navigation for React DOM and React Native. Products supply routes,
copy, icon renderers and tokens. The package imports no router, icon library or
Tailwind code.

## Entries

- `@baukit/navigation` contains the TypeScript model, validation, active-route
  resolution, section rotation, disclosure reducer and layout selection. It has
  no React import.
- `@baukit/navigation/web` contains `AppNavigation` and `SectionPicker` for DOM.
  It never imports React Native, including through `a11y-core`.
- `@baukit/navigation/native` contains those components for React Native and
  React Native Web. Use `AppNavigation` as an Expo Router custom `tabBar`.
- `@baukit/navigation/web.css` styles the DOM entry with `--bk-*` variables.

```tsx
import { AppNavigation, SectionPicker } from '@baukit/navigation/web';
import '@baukit/navigation/web.css';

const progress = {
  id: 'progress', label: 'Progress', href: '/progress',
  icon: ({ active, size }) => <ChartIcon filled={active} size={size} />,
  children: [
    { id: 'overview', label: 'Overview', href: '/progress' },
    { id: 'history', label: 'History', href: '/progress/history' },
  ],
};

<AppNavigation
  items={[progress]}
  pathname={pathname}
  onNavigate={(href) => router.navigate(href)}
  profile={{ label: 'Account', initials: 'AB', href: '/profile' }}
/>;
<SectionPicker item={progress} pathname={pathname} onNavigate={navigate} />;
```

Declare `items` as `readonly NavigationItem<NavigationIcon>[]` to type icon
parameters. Web and native entries each export `NavigationIcon`. Renderers
receive `{ active, size }`. The wrapper hides decorative icons from assistive
technology. Include section roots in `children` when they belong in the cycle.

The default route match is an exact href or a slash-delimited descendant.
`matches(pathname)` replaces it. Children win over parents, and the longest
matching href wins among children. Pass a custom matcher for route aliases,
search parameters or hashes. A missing match selects nothing. `nextSectionHref`
returns the first child for a missing match and wraps at the last child.

Below 1024 pixels the component renders a bottom bar with icons above labels.
It supports five main items plus an optional profile. Keep labels short enough
for the compact bar. At 1024 pixels it renders a 280-pixel rail. Collapse reduces
it to 76 pixels. The active section starts open. Clicking a collapsed disclosure
expands the rail and opens that section.

`collapsed`, `defaultCollapsed`, and `onCollapsedChange` support controlled and
uncontrolled state. Persist the controlled value in the product's preference
store. Use `collapseLabel`, `expandLabel`, and `label` for translated copy.

Web uses real anchors. `onNavigate` intercepts only an unmodified primary click.
Without it the browser follows the href. `renderLink(props)` can return a router
Link. Forward every supplied prop, especially href, ref, onKeyDown, onClick and
ARIA attributes. Keep modified clicks in the browser. TanStack Router products
can map `href` to `to`, or use `onNavigate` with their router instance.

A profile accepts either `href` or a nonempty `menu`. Menu entries have unique
ids, labels and either `href` or synchronous `onSelect`. A product owns async
action errors. Start an async sign-out in `onSelect` and handle its rejection in
the product. `imageUrl` renders an avatar, with initials after an image failure.
The profile stays last in the bar and at the bottom of the rail.

## Native tokens and shell

Supply a `NavigationTheme` from the product's compiled ui-tokens. Do not use
raw brand colors in the navigation component.

| NavigationTheme field | Semantic token |
| --- | --- |
| background | color.background.primary or surface |
| text | color.text.primary |
| muted | color.text.muted |
| activeBackground | color.background.accent |
| activeText | color.text.onAccent |
| border | color.border.primary |
| focus | color.focus.ring |
| spacing | space.small |
| radius | radius.small |

Use a tested contrast pair for `activeText` and `activeBackground`. Native
changes are immediate. Web width transitions run only after reduced motion
resolves and only if motion is allowed.

Set Expo Tabs' `tabBarPosition` to `left` in rail mode and `bottom` otherwise.
Pass the safe-area insets and pathname to the custom tab bar. The component
occupies layout space on native. The DOM component is fixed; reserve its width
or bottom height in the content shell. `NAVIGATION_DIMENSIONS` exports those
numbers. Match the shell's margin and width transitions to the rail, or reserve
the expanded width. Enable transitions only for `data-motion="standard"`.
Include the bottom safe-area inset. The section picker belongs inside
the current section's content.

Keep route-heading focus in the product. Use
`createRouteFocusController` from `@baukit/a11y-core/web` for DOM route changes,
and the product's screen-transition adapter on native. Navigation does not know
which heading has mounted.

The DOM stylesheet uses stable `bk-navigation*` classes and `data-layout`,
`data-collapsed`, `data-active`, `data-open`, and `data-motion` attributes.
Products can override these selectors in plain CSS or Tailwind.

## Verification

`pnpm test` runs model and DOM tests, native tests with React Native Testing
Library and the React Native Jest preset, and checks the packed exports. Expo
is not needed for these tests. `pnpm test:browser` runs Playwright through
Vitest in Chromium and WebKit at 320, 1023 and 1024 pixels, with short and normal
heights. It checks geometry, 44-pixel targets, browser warnings and axe.
