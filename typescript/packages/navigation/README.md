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
  id: 'progress',
  label: 'Progress',
  href: '/progress',
  icon: ({ active, size }) => <ChartIcon filled={active} size={size} />,
  children: [
    { id: 'overview', label: 'Overview', href: '/progress' },
    { id: 'history', label: 'History', href: '/progress/history' },
  ],
};

<AppNavigation
  label={copy.primary}
  collapseLabel={copy.collapse}
  expandLabel={copy.expand}
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

The default route match compares paths and ignores query strings and hashes. It accepts an exact path or a slash-delimited descendant.
`matches(pathname)` replaces it. Children win over parents, and the longest
matching href wins among children. Pass a custom matcher for route aliases,
search parameters or hashes. A missing match selects nothing. `nextSectionHref`
returns the first child for a missing match and wraps at the last child.

Below 1024 pixels the component renders a bottom bar with icons above labels.
It supports five main items plus an optional profile. Keep labels short enough
for the compact bar. Longer labels ellipsize inside their buttons; accessible labels retain the full text. At 1024 pixels it renders a 280-pixel rail. Collapse reduces
it to 76 pixels. The active section starts open. Clicking a collapsed disclosure
expands the rail and opens that section.

`collapsed`, `defaultCollapsed`, and `onCollapsedChange` support controlled and
uncontrolled state. Persist the controlled value in the product's preference
store. Supply `collapseLabel`, `expandLabel`, and `label` from the product's catalog. Native `AppNavigation` and `SectionPicker` also require `closeLabel` for the visible and accessible menu Close control. Components have no English defaults.

Web uses real anchors. `onNavigate` intercepts only an unmodified primary click.
Without it the browser follows the href. `renderLink(props)` can return a router
Link. Forward every supplied prop, especially href, ref, onKeyDown, onClick and
ARIA attributes. Keep modified clicks in the browser. TanStack Router products
can map `href` to `to`, or use `onNavigate` with their router instance.

A profile accepts either `href` or a nonempty `menu`. Menu entries have unique
ids, labels and either `href` or synchronous `onSelect`. Link entries accept `matches(pathname)`. Only the most-specific matching path is selected in a menu. A product owns async
action errors. Start an async sign-out in `onSelect` and handle its rejection in
the product. `imageUrl` renders an avatar, with initials after an image failure.
The profile stays last in the bar and at the bottom of the rail.

Active leaves use a muted accent background with an accent icon and bold label.
In the expanded rail, the parent of an active child has accent text and icon
without a fill. The active icon renderer receives `active: true` for both rows.
In the bar and collapsed rail, the parent gets the fill. Web rows expose
`data-active="page"` or `data-active="ancestor"`; inactive rows omit the attribute.
Override `--bk-navigation-active-background`, `--bk-navigation-active-text` and
`--bk-navigation-ancestor-text` on `.bk-navigation` to match a product palette.
The default background mixes 14% accent over the navigation background.

## Native tokens and shell

Supply a `NavigationTheme` from the product's compiled ui-tokens. Do not use
raw brand colors in the navigation component.

| NavigationTheme field | Semantic token                                       |
| --------------------- | ---------------------------------------------------- |
| background            | color.background.primary or surface                  |
| text                  | color.text.primary                                   |
| muted                 | color.text.muted                                     |
| activeBackground      | blendColors(accent, background, 0.14) from ui-tokens |
| activeText            | color.background.accent                              |
| ancestorText          | color.background.accent                              |
| border                | color.border.primary                                 |
| focus                 | color.focus.ring                                     |
| spacing               | space.small                                          |
| radius                | radius.small                                         |

Use a tested contrast pair for `activeText` and the muted `activeBackground`.
Check `ancestorText` against `background` too. `blendColors` and `contrastRatio`
from ui-tokens check these pairs in both themes. Native
changes are immediate. Web width transitions run only after reduced motion
resolves and only if motion is allowed.

Set Expo Tabs' `tabBarPosition` to `left` in rail mode and `bottom` otherwise.
Pass the safe-area insets and pathname to the custom tab bar. The component
occupies layout space on native. The DOM component is fixed; reserve its width
or bottom height in the content shell. `NAVIGATION_DIMENSIONS` exports those
numbers, including 44-unit web and iOS targets and 48 dp Android targets. Match the shell's margin and width transitions to the rail, or reserve
the expanded width. Enable transitions only for `data-motion="standard"`.
Include the bottom safe-area inset. The section picker belongs inside
the current section's content. The DOM picker measures the space below its
trigger and above the compact bottom bar. Long sections scroll inside that
space. It updates the limit when the viewport, content layout, or scroll
position changes.

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
