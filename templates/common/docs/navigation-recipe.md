# Navigation recipe

Use `@baukit/navigation` for primary navigation and section pickers. Products supply routes, icons, copy and tokens. The shared package keeps the rules below.

## Measurable rules

- Give each target an effective hit area of at least 44 by 44 CSS pixels on web and 44 by 44 points on iOS and 48 by 48 dp on Android.
- Show a visible keyboard focus indicator and a separate selected state. On web, set `aria-current="page"` on the active route. On native, expose the selected state through the accessibility API.
- Pair every icon with an accessible label. Hide decorative icons when adjacent text already supplies the label.
- Use a compact bottom bar below 1024 CSS pixels and a wide rail at 1024 CSS pixels and above. Keep destination order and labels stable across the boundary.
- Reserve layout space for fixed navigation. Content, actions, focused controls, and scrollbars must not overlap it. Include safe-area insets in both fixed navigation and content padding.

## Web links

Render destinations as anchors with real `href` values. Router code may intercept an unmodified primary-button click. Leave Ctrl-click, Command-click, Shift-click, Alt-click, and middle-click to the browser.

## Profile menu

Open the menu with a button that has `aria-haspopup="menu"` and an accurate `aria-expanded` value. Move focus to the first enabled item when keyboard input opens it. Arrow keys move between enabled items. Home and End move to the first and last enabled item. Enter and Space activate the focused item. Escape closes the menu and restores focus to its trigger. Tab closes the menu and continues through the document.

Focus and selection are separate. Focus marks the next action. Selection marks the current destination or account.

## Route focus

The web route focus controller from `@baukit/a11y-core/web` accepts a getter for the route heading. It retries while the heading mounts, stops if the user focuses another control, and restores the initiating control on exit. It is DOM-only. The generated mobile adapter calls it through Expo Router's focus lifecycle for Expo web and keeps native headings exposed with `accessibilityRole="header"`.

## Reduced motion

Read `{ reducedMotion, resolved }` from `useReducedMotionPreference()`. Do not start rail or sheet transitions before the preference resolves. When reduced motion is active, do not slide, scale, or spring between the compact bar and wide rail. Open sheets without spatial motion. Focus moves at the same logical point in both modes.

## Browser evidence

Test widths 320, 1023, and 1024 with short and normal heights. The generated Playwright helpers check horizontal overflow, fixed-navigation overlap, scroll-container containment, and 44-by-44 CSS-pixel targets. The console check rejects every warning outside an exact allowlist. Each allowlist entry must state why it is safe.

## Shared component integration

Import models from `@baukit/navigation`, DOM components from
`@baukit/navigation/web`, and React Native components from
`@baukit/navigation/native`. Import `@baukit/navigation/web.css` in DOM apps.
The web entry never loads React Native. Pass `pathname` and `onNavigate` to
connect any router. Use `renderLink` only when a router needs its Link component,
and forward the supplied anchor and focus props.

Supply unique ids, labels, hrefs and icon renderers for main items. Children
have their own ids, labels and hrefs. Include the section root in its children
when it belongs in the rotation. `matches` can recognize aliases or nested
routes. Without it, a route matches its href and slash-delimited descendants.
The longest matching child wins. A missing match selects nothing and starts
rotation at the first child.

The rail expands to 280 pixels and collapses to 76 pixels. Main icons sit left
of labels. Collapsed rows retain accessible names and web titles. A section
with children is a disclosure button with `aria-expanded` and `aria-controls`.
Its children follow it directly, indented beside a guide line. The active group
starts open. Clicking a collapsed disclosure expands the rail and opens that
group. Active leaves use a muted accent background, accent text and a bold
label. The parent of an active child has accent text and icon without a fill.

Compact bars keep icons above short labels. An inactive tab opens its root; an
active tab rotates to the next child and wraps. `SectionPicker` presents every
child for direct selection through a full-width section and current-page
trigger. Keep profile last in both layouts. A profile accepts initials, an
optional image, and either an href or a product-provided menu. Handle async menu
action failures in the product.

Control and persist collapse with `collapsed` and `onCollapsedChange`, or use
`defaultCollapsed` for local state. DOM navigation is fixed, so reserve rail
width and bar height in content. Native navigation occupies its container;
Expo Tabs must position the custom tabBar left for expanded layouts and bottom
otherwise. Pass safe-area insets. Native `NavigationTheme` maps background,
text, muted, activeBackground, activeText, ancestorText, border and focus to the corresponding
semantic color tokens, plus spacing and radius to compiled numeric tokens.
Blend the accent over the navigation background at 14% for activeBackground.
Use the accent for activeText and ancestorText. Check activeText against
activeBackground and ancestorText against background in both themes with
ui-tokens contrast helpers.

The package's browser suite checks Chromium and WebKit at 320, 1023 and 1024
pixels and heights 568 and 720. Keep product checks for actual primary actions,
scroll containers, console messages and screen-reader behavior.

On native, the compact bar reserves `64 * max(1, fontScale) + bottomInset`.
Use `useNavigationBarHeight(bottomInset)` from `@baukit/navigation/native` for
content padding outside a navigator that measures its tab bar.
