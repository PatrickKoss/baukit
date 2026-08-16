# Accessibility contract

This contract is the Baukit baseline for generated web and React Native products. It defines behavior and evidence, not a component library. Products keep their components, navigation, copy, graphics, and visual design local.

## Screen and interaction baseline

Every supported screen and state must expose meaningful names, roles, values, state, reading order, and headings without depending on color, position, shape, animation, or a gesture alone.

### Overlays and focus

- On web, move focus into an opened modal, contain Tab and Shift+Tab, close on Escape when dismissal is allowed, make the background inert, and restore focus to the stable trigger.
- On native, require the caller to provide stable refs for the trigger and the overlay container or initial heading. Move accessibility focus into the overlay only after that target has laid out. Restore focus to the supplied trigger when the overlay closes and the trigger still exists; otherwise focus an intentional, stable destination.
- Mark native overlay content with `accessibilityViewIsModal` on iOS. Hide the app content _behind_ an open overlay with `accessibilityElementsHidden` on iOS and `importantForAccessibility="no-hide-descendants"` on Android. These background-hiding props belong on the background content container, not on the overlay itself.
- Do not assume that React Native `Modal`, visual dimming, `aria-hidden`, or z-order alone removes background content from the accessibility tree. Verify traversal on each platform.

A generic native hook cannot reliably discover which element previously held accessibility focus. It must not guess. Callers own the stable trigger/container refs and the fallback destination. A layout callback or equivalent native presentation event is part of the focus-entry contract; tests must prove that no focus request occurs before layout.

### Announcements and state changes

Announce important asynchronous outcomes that are not otherwise reached naturally, such as a completed save, a failed refresh, or a newly available result. Use an `aria-live`/status or alert region on web and `AccessibilityInfo.announceForAccessibility` on native. Keep announcements short, localized, and specific; do not announce every render or duplicate visible focus changes.

Loading, error, empty, and ready states must be mutually understandable. Preserve stable machine codes outside user-facing copy and give errors a recovery action when one exists.

### Reduced motion

Honor the web reduced-motion media query and `AccessibilityInfo.isReduceMotionEnabled()` on native, including preference changes during a running session when the product supports them. Remove non-essential motion, use an immediate or short cross-fade alternative, and never make animation the only signal of a state change. Essential progress and spatial context may remain, but must avoid unnecessary looping, parallax, flashing, or large movement.

### Targets, forms, and graphics

- Give web controls at least a 44 by 44 CSS-pixel effective target. Give native controls at least 44 by 44 points on iOS and 48 by 48 dp on Android, using `hitSlop` when the visible control should remain smaller. Keep adjacent targets separable.
- Associate every form label, help message, required state, and error with its field. Set invalid state, announce a submit summary or the first new error, and move focus to the first invalid field after submit. Do not clear user input after a failed submission.
- Give charts, heatmaps, maps, progress rings, and other data graphics an accessible name plus a non-visual alternative containing the decision-relevant values, units, time range, trend, and exceptional states. Provide an accessible table or structured summary when individual values matter. Product-specific grouping and summary copy stay in the product.

## Evidence layers

No single layer is sufficient:

1. **Unit seams:** assert native iOS and Android props separately, announcement events, reduced-motion reads/changes, and focus requests after layout. Assert web dialog focus/inert behavior and semantic error linkage.
2. **Static lint:** run the generated mobile React Native accessibility ESLint rules with zero warnings, alongside strict TypeScript lint.
3. **Automated tree scan:** run axe against the generated web app and accessible dialog example and fail on serious or critical violations. Extend scans to changed product states.
4. **Device pass:** complete the VoiceOver and TalkBack protocol below on a release binary and record the environment and results.

Prop assertions find wiring regressions; axe finds detectable DOM failures. Neither evaluates announcement timing, spoken meaning, focus order, gesture behavior, native platform differences, or whether a graphic summary is useful. They cannot replace a real VoiceOver/TalkBack pass.

## VoiceOver and TalkBack release protocol (version 1)

Run this protocol for each native release candidate and after navigation, overlay, form, graphics, or accessibility infrastructure changes. Use a representative supported iOS device with VoiceOver and Android device with TalkBack. Test the release binary with production-like fonts, localization, and data; do not treat Jest, a browser preview, or the Expo development shell as the device result.

1. Record the app build, device model, OS, screen-reader version, locale, text-size setting, reduced-motion setting, and tester.
2. Start from a cold launch. Traverse every element on each critical screen using only screen-reader gestures. Check names, roles, values, states, headings, reading order, adjustable controls, and that no decorative element becomes a stop.
3. Activate every critical action without sight. Confirm targets are reachable, do not overlap, and have an alternative to custom or multi-finger gestures.
4. Open and close each overlay from a known trigger. Confirm entry focus occurs after presentation, traversal cannot reach background content, dismissal is understandable, and focus returns to the trigger or documented fallback.
5. Submit each representative form empty, invalid, and valid. Confirm errors are linked and announced once, focus reaches the first invalid field, entered values survive failures, and success is announced.
6. Exercise loading, empty, error, retry, offline, and completed states. Confirm important changes are announced without repeated or interrupted speech.
7. Enable reduced motion and repeat every animated critical transition. Confirm the alternative preserves state and task understanding.
8. Traverse every data graphic. Confirm its summary/table communicates the values, units, range, trend, and exceptions needed to complete the task.
9. Record each failure with reproduction steps and evidence. A claimed native accessibility baseline passes only when all blocking failures are fixed and both platform results are attached to the release record.

Use this results record:

```text
Accessibility protocol: v1
App/build:
Date/tester:

Platform: iOS | Android
Device/model:
OS version:
Screen reader/version:
Locale/text size:
Reduce Motion setting:

Critical paths covered:
Overlays covered:
Forms and dynamic states covered:
Data graphics covered:

Result: PASS | FAIL | BLOCKED
Findings (severity, screen, steps, expected, actual, evidence):
Accepted limitations and owner:
Follow-up issue/release:
```

Keep one completed record per platform with the release evidence. `BLOCKED` is not a passing result.
