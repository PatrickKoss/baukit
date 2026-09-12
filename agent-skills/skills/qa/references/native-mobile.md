# Native mobile QA

Use the application's native release build when the audit covers Android or iOS. A responsive browser viewport does not exercise native navigation, storage, permissions, keyboards, safe areas, deep links, or accessibility services.

## Prefer repository targets

Inspect the repository's Makefile and mobile documentation first. In a Baukit-generated product, use the platform-specific targets from the repository root:

```sh
make qa-android
make e2e-android-live
make qa-android-down

make qa-ios
make e2e-ios-live
make qa-ios-down
```

`qa-android` and `qa-ios` leave the device and disposable services running for exploratory work. `e2e-android-live` and `e2e-ios-live` run the repository's Maestro flows against that device. Use `e2e-android` or `e2e-ios` when one automated run with cleanup is enough. Mobile-only products expose the targets through `make -C mobile`.

Do not replace project targets with ad hoc Expo commands unless the targets are broken or absent. They encode the expected build type, service ports, package identifiers, native configuration, test accounts, and cleanup behavior.

## Platform requirements

Android emulator testing works on Linux and macOS when hardware virtualization, Java, and the Android SDK are available. Baukit's setup target installs its pinned SDK components and creates a dedicated QA AVD.

iOS Simulator testing requires macOS, Xcode, an installed iOS Simulator runtime, CocoaPods, and Node.js with Corepack. It does not require EAS. A local Simulator build is free apart from the Mac and developer time. Do not report iOS as passed when the audit ran only on Linux, Android, Expo web, or a browser's mobile viewport.

Maestro must be installed for automated flows. The interactive targets still support manual testing without it.

## Test sequence

1. Record the host OS, Xcode or Android API version, simulator or emulator model, app build type, and commit.
2. Start the platform environment and run the existing Maestro smoke flow before exploratory work. Treat a failed baseline as a finding or blocker, not as skipped coverage.
3. Exercise platform Back behavior, app termination and relaunch, direct deep links, offline transitions, permission denial, keyboard avoidance, safe areas, rotation where supported, and persistence.
4. Repeat critical workflows after a cold launch. Check both the UI result and stored or server state.
5. Capture screenshots and platform logs for findings. Keep generated artifacts out of version control unless the user asks to commit them.
6. Stop the platform environment when the audit ends. Confirm that the target did not stop an unrelated emulator or remove normal development data.

The generated release build embeds its JavaScript, so Metro is not part of the result. Do not leave Metro running and assume the installed app is using it.

## Accessibility limits

Use TalkBack on Android and VoiceOver on iOS for important workflows when the environment supports them. Verify spoken names, traversal order, focus after navigation and overlays, announcements, adjustable controls, and keyboard or switch access where applicable.

Simulator and emulator checks are useful, but they do not replace the project's physical-device release protocol. Record screen-reader checks on a simulator separately from physical-device evidence, and state the remaining gap in the audit report.
