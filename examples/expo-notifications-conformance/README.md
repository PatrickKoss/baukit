# Hermes vectors and expo-notifications device conformance

This Expo SDK 57 app runs two checks inside Hermes on an Android emulator.

- The shared vectors. `fixtures/zoned-time/vectors-v1.json` goes through
  `zonedTimeVectorChecks` from `@baukit/localization-core/vectors`, and
  `fixtures/notifications/plan-vectors-v1.json` through `notificationPlanVectorChecks` from
  `@baukit/notifications-core/vectors`. These are the same checks the packages' Vitest suites run
  on Node, so a mismatch here is a disagreement between Hermes and Node. Each mismatch is logged
  as `BAUKIT_HERMES_VECTOR_MISMATCH` with its label, actual, and expected value.
- `@baukit/notifications-expo` against real `expo-notifications`. With `POST_NOTIFICATIONS`
  granted, the app schedules owned requests, reruns the replacement with one kept, one changed,
  one removed, and one new entry, reruns it unchanged, and clears it. A foreign request and a
  request of the prefix-sibling namespace `reminders-extra` must survive every step. With the
  permission revoked, the replacement must report `permission_denied` for every entry, schedule
  nothing, and still cancel an owned request left from a granted period.

On Linux with Java 21 and KVM available:

```sh
make expo-notifications-conformance
```

The run script launches the app twice. It grants `POST_NOTIFICATIONS` with `adb shell pm grant`
and waits for `BAUKIT_HERMES_VECTORS_PASS` and `BAUKIT_NOTIFICATIONS_GRANTED_PASS`. Then it
revokes the permission, marks it `user-fixed` so the runtime request answers without a prompt, and
waits for `BAUKIT_NOTIFICATIONS_DENIED_PASS`. Any `BAUKIT_NOTIFICATIONS_CONFORMANCE_FAIL` fails the
run. Emulator boot, install, and Metro come from `scripts/expo-android-conformance.sh`, which the
SQLite conformance app shares. Set `METRO_PORT` to move Metro off 8081. Diagnostics are kept in
`artifacts/`.

The app imports the fixtures from the repository through `metro.config.cjs`, which adds
`../../fixtures` to Metro's watch folders.

This is an emulator run, not a physical device. iOS needs macOS with Xcode and is not covered.
