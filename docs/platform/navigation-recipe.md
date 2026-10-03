# Navigation recipe

Use `@baukit/navigation` for shared web and mobile navigation. [ADR 0005](../adr/0005-shared-application-navigation.md) replaces the earlier product-owned navigation boundary. Products still choose routes, labels, icons, semantic tokens and content layout.

## Interaction rules

- Give every navigation target an effective hit area of at least 44 by 44 CSS pixels on web and 44 by 44 points on native. Measure the rendered rectangle, including any padding on the interactive element.
- Show a visible keyboard focus indicator. Do not use color alone for the selected state. On web, set `aria-current="page"` on the active route. On native, expose the selected state through the platform accessibility API.
- Pair each icon with an accessible label. Hide a decorative icon from the accessibility tree when adjacent text already supplies the label.
- Use a compact bottom bar below 1024 CSS pixels and a wide rail at 1024 CSS pixels and above. Test 1023 and 1024 directly. Keep primary destinations in the same order and preserve their labels when the layout changes.
- Reserve layout space for fixed navigation. Content, actions, focused controls, and scrollbars must not sit under the bar or rail. Add the bottom safe-area inset to compact navigation and its content padding. Add side insets when a rail touches a device edge.

## Web links

Render navigation destinations as anchors with real `href` values. A router may intercept an unmodified primary-button click. It must leave Ctrl-click, Command-click, Shift-click, Alt-click, and middle-click to the browser. Those actions open tabs or windows and must keep working.

Use buttons for actions that do not navigate. Styling a `div` as a link loses browser link behavior and requires a fragile keyboard imitation.

## Profile menu

Use a button to open the profile menu. Give it `aria-haspopup="menu"`, keep `aria-expanded` in sync, and connect it to the menu with `aria-controls` when the menu is mounted.

When the menu opens from the keyboard, focus its first enabled item. Arrow Down and Arrow Up move through enabled items. Home and End move to the first and last enabled item. Enter and Space activate the focused item. Escape closes the menu and restores focus to its trigger. Tab closes the menu and continues through the document instead of trapping focus. A pointer click outside closes it without moving focus to an unrelated element.

Keep the selected account or destination exposed independently of focus. Focus tells the user where the next action occurs. Selection tells them which state is current.

## Route focus

On web, call `createRouteFocusController()` from `@baukit/a11y-core/web` once and enter a route with a getter for its level-one heading. The controller retries while a heading mounts, avoids stealing focus after the user moves it, and restores the initiating control when the route exits. The controller is DOM-only.

Expo Router products can call the controller from `useFocusEffect` for Expo web. Pass a stable heading ref and delay entry until the heading is ready. Keep native headings marked with `accessibilityRole="header"`; native screen-reader focus needs a product-owned adapter tied to the screen transition.

## Reduced motion

Read `{ reducedMotion, resolved }` from `useReducedMotionPreference()`. Before `resolved` becomes true, render a stable state instead of starting a transition that may need to be cancelled.

When reduced motion is active, switch the compact bar and wide rail without sliding, scaling, or spring motion. Open and close sheets without a spatial transition. An immediate state change is acceptable. If opacity helps preserve context, keep it brief and do not combine it with movement. Focus must move at the same logical point with or without animation.

## Browser checks

At minimum, run layout checks at 320, 1023, and 1024 CSS pixels. Include a 568-pixel short height and a normal 720-pixel height for each width. Assert these facts:

- the document has no horizontal overflow;
- a primary action does not intersect fixed navigation;
- a target stays inside its scroll container;
- every visible interactive target is at least 44 by 44 CSS pixels.

Collect browser console warnings during the same routes. Allow only exact messages, and record a reason for every exception. A changed suffix or extra detail is a new warning and must fail the check.

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
