# Navigation recipe

Use this recipe when the product adds primary navigation or a profile menu.

## Measurable rules

- Give each target an effective hit area of at least 44 by 44 CSS pixels on web and 44 by 44 points on native.
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
