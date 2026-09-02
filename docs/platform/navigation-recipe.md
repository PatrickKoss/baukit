# Navigation recipe

Use this recipe for product-owned web and mobile navigation. Baukit supplies focus and reduced-motion behavior, but products still choose routes, labels, icons, and layout.

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
