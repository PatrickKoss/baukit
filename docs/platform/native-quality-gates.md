# Native quality gates

**Status:** Platform contract for generated Expo SDK 57 products

Native support means more than TypeScript correctness. Config plugins, native
modules, manifests, Gradle/Xcode settings, and device storage are exercised in
layers so the quickest evidence arrives first without turning an unavailable
runner into a false pass.

## Gate layers

| Layer | When | Baukit-generated default | Product responsibility |
|---|---|---|---|
| TypeScript, ESLint, Jest | Every relevant product change | Blocking `mobile` job | Add product behavior tests |
| Jest coverage thresholds | Every relevant product change | Blocking `mobile-coverage` job; floors in `mobile/jest.config.cjs` | Raise the floors as the product grows |
| Android native compile | Pull requests and `main` changes under `mobile/` or its workflow/config | Blocking clean Expo prebuild plus Gradle `assembleDebug` | Keep native plugins and configuration compilable |
| Maestro critical paths | When product-owned `.maestro/` journeys exist | Configurable in the manual/scheduled native workflow | Define stable, focused journeys and required backend/auth fixtures |
| iOS Simulator compile | Weekly schedule or manual dispatch | Configurable `macos-15` job; not a Linux PR gate | Fund/enable the macOS runner and investigate failures |
| VoiceOver and TalkBack | Before a release claims native accessibility | Protocol and result format, not automation | Record physical device, OS, build, operator, and findings |

The generated `.github/workflows/ci.yml` keeps source checks blocking. Which
jobs it contains follows the capabilities the product was generated with:
`backend`, `backend-msrv`, `api-drift`, and `docker-build` for a backend;
`web`, `web-coverage`, and the `e2e-web` browser matrix for a web app;
`mobile` and `mobile-coverage` for a mobile app; `observability-lint` always.
The
generated `.github/workflows/native.yml` is path-filtered, cancels superseded
runs, caches pnpm and Gradle downloads, uploads failure diagnostics, and makes
Android compilation blocking whenever its relevant trigger fires. Its weekly
iOS job compiles on a real macOS runner. Maestro runs only when manual input or
`RUN_MAESTRO_NATIVE_GATE=true` requests it; requesting Maestro without a
product-owned `mobile/.maestro/` directory fails.

Baukit itself additionally compiles a freshly generated mobile fixture and
runs the Expo SQLite conformance app on an Android emulator when the adapters,
template, CLI, or relevant dependencies change.

## Skipped is not green

A path-excluded job is explicitly **not applicable**; it makes no native claim.
Once relevant paths select a native gate, a missing runner, SDK, credentials,
billing, emulator, or test journey is a failed or blocked result. Required
native jobs must not use `continue-on-error`, conditionally replace work with a
successful no-op, or report a Linux source check as an Android/iOS pass.

Baukit CI uses a final gate job to distinguish “not applicable” from an
attempted build that was skipped, cancelled, blocked, or failed. Products
should protect the Android workflow check on branches where mobile changes are
merged.

## Cost and runner requirements

The Android compile requires Linux, Java 21, the Android API 36 SDK, and Gradle;
it does not require an emulator. The real SQLite proof additionally requires
hardware virtualization and an API 36 x86_64 emulator image. Locally,
`scripts/android-sdk-setup.sh` installs only those components under
`$HOME/Android/Sdk`; `make native-android-gate` compiles a clean generated
fixture and `make expo-sqlite-conformance` boots the emulator.

iOS requires macOS, Xcode, CocoaPods as selected by Expo, and available
Simulator runtime capacity. It is deliberately scheduled/manual because its
runner is slower and more expensive. A developer on Linux or WSL2 must record
iOS as **blocked locally: requires macOS/Xcode**, never passed or skipped-green.

## Smoke and release evidence

Native compile proves linkage, not behavior. Product smoke coverage should
open at least one custom-scheme deep link from a terminated app and verify the
expected valid, invalid, and unauthenticated route outcomes. With the keyboard
open, verify focused controls remain visible, dismissal does not lose input,
and content/overlays respect top and bottom safe-area insets in portrait and
landscape where supported.

Route names, OAuth callbacks, screen wrappers, test accounts, backend topology,
and Maestro flows stay product-owned. Before release, combine automated results
with the physical VoiceOver/TalkBack protocol in the
[accessibility contract](./accessibility-contract.md).
