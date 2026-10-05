# Changelog

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.
- Updated dependencies
  - @baukit/a11y-core@0.7.2
  - @baukit/ui-tokens@0.7.2

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.
- Updated dependencies
  - @baukit/a11y-core@0.7.1
  - @baukit/ui-tokens@0.7.1

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/a11y-core@0.7.0
  - @baukit/ui-tokens@0.7.0

## [Unreleased]

- Add `renderAvatar` for product profile glyphs and frames on web and native. Keep profile labels, subtitles and menu focus.

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

- Add optional profile subtitles on web and native rails and menus. Include the subtitle in the profile button label.
- Add optional native font family, sizes, weights, line height and letter spacing. Apply them to navigation labels, menus and section pickers.
- Present native menus in a Modal on web so compact navigation cannot clip them. Bound popups to the viewport.
- Move native accessibility focus after Modal presentation. Hide background navigation from screen readers while the menu is open.
- Start section picker keyboard focus on its selected child.

- Require product labels for navigation, collapse, expand, and native menu Close controls. Remove English defaults.
- Give every native target a 48 dp minimum on Android and 44 pt on iOS.
- Expose native picker and profile menu selection state and highlight the current entry.
- Ignore query strings and hashes when matching routes, unless an entry supplies `matches`.
- Select one most-specific profile menu entry and accept custom `matches` callbacks.
- Bound compact bar labels to their buttons and ellipsize long text at narrow widths.

- Draw web focus rings 2 pixels outside controls. Reserve room for rings in collapsed rails and compact bars. Bound section menus above the bottom bar and scroll long sections inside the menu.

- Resolve lazy native modules during test setup so cold compilation stays outside each render test's timer.

- Add shared navigation models, route matching and compact section rotation.
- Add collapsible React DOM and React Native rails, bottom bars, profile menus
  and section pickers.
- Add keyboard, native and real-browser conformance tests.
