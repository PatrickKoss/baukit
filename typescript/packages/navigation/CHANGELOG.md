# Changelog

## [Unreleased]

## 0.10.0

- Remove web menus before restoring focus and calling menu actions or navigation callbacks.

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/a11y-core@0.10.0
  - @baukit/ui-tokens@0.10.0

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/a11y-core@0.9.0
  - @baukit/ui-tokens@0.9.0

## 0.8.0

- Run native menu callbacks after dismissal. Products can pass navigation and sign-out callbacks directly.
- Add a danger tone for menu entries, with semantic token colors on web and native.
- Require localized visible Close controls in web profile and section menus. Keep Close visible while long menus scroll.
- Add brand and accessory render slots for rails and compact bars. Report compact height with slots for content spacing.
- Wrap long web slot and menu labels within their containers.

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/a11y-core@0.8.0
  - @baukit/ui-tokens@0.8.0

## 0.7.4

- Fill the compact viewport and allow horizontal scrolling when items overflow. Reveal the selected item after route changes. Keep decorative initials inside their fixed avatar circle at enlarged font scales.
- Give compact icons and avatars the same box height so their labels align at every font scale.
- Apply the body font to standalone web section pickers.
- Derive compact native bar height from the avatar, icon, label line height, spacing, padding and borders. Size compact icon containers from the same dimensions so Android font padding cannot squeeze labels. Keep rendered height and reserved height aligned at normal and enlarged font scales.

### Patch Changes

- Release the coordinated baukit 0.7.4 train.
- Updated dependencies
  - @baukit/a11y-core@0.7.4
  - @baukit/ui-tokens@0.7.4

## 0.7.3

- Reserve compact native bar height for enlarged text and safe areas. Expose `getNavigationBarHeight` and `useNavigationBarHeight` for content padding.
- Select the visible parent for keyboard entry when the active child is hidden.
- Publish a CSS declaration so side-effect imports type-check without product declarations. Document the Android 48 dp target minimum.
- Add `renderAvatar` for product profile glyphs and frames on web and native. Keep profile labels, subtitles and menu focus.
- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.
- Add optional profile subtitles on web and native rails and menus. Include the subtitle in the profile button label.
- Add optional native font family, sizes, weights, line height and letter spacing. Apply them to navigation labels, menus and section pickers.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.
- Updated dependencies
  - @baukit/a11y-core@0.7.3
  - @baukit/ui-tokens@0.7.3

## 0.7.2

- Present native menus in a Modal on web so compact navigation cannot clip them. Bound popups to the viewport.
- Move native accessibility focus after Modal presentation. Hide background navigation from screen readers while the menu is open.
- Start section picker keyboard focus on its selected child.
- Require product labels for navigation, collapse, expand, and native menu Close controls. Remove English defaults.
- Give every native target a 48 dp minimum on Android and 44 pt on iOS.
- Expose native picker and profile menu selection state and highlight the current entry.
- Ignore query strings and hashes when matching routes, unless an entry supplies `matches`.
- Select one most-specific profile menu entry and accept custom `matches` callbacks.
- Bound compact bar labels to their buttons and ellipsize long text at narrow widths.

### Patch Changes

- Release the coordinated baukit 0.7.2 train.
- Updated dependencies
  - @baukit/a11y-core@0.7.2
  - @baukit/ui-tokens@0.7.2

## 0.7.1

- Draw web focus rings 2 pixels outside controls. Reserve room for rings in collapsed rails and compact bars. Bound section menus above the bottom bar and scroll long sections inside the menu.
- Resolve lazy native modules during test setup so cold compilation stays outside each render test's timer.

### Patch Changes

- Release the coordinated baukit 0.7.1 train.
- Updated dependencies
  - @baukit/a11y-core@0.7.1
  - @baukit/ui-tokens@0.7.1

## 0.7.0

- Add shared navigation models, route matching and compact section rotation.
- Add collapsible React DOM and React Native rails, bottom bars, profile menus
  and section pickers.
- Add keyboard, native and real-browser conformance tests.

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/a11y-core@0.7.0
  - @baukit/ui-tokens@0.7.0
