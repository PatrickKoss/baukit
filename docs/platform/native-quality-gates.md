# Native quality gates

**Status:** Platform contract for generated Expo SDK 57 products

Native support means more than TypeScript correctness. Config plugins, native
modules, manifests, Gradle/Xcode settings, and device storage are exercised in
layers so the quickest evidence arrives first without turning an unavailable
runner into a false pass.

## Gate layers

| Layer                    | When                                                                                | Baukit-generated default                                                       | Product responsibility                                                        |
| ------------------------ | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| TypeScript, ESLint, Jest | Every relevant product change                                                       | Blocking `mobile` job                                                          | Add product behavior tests                                                    |
| Jest coverage thresholds | Every relevant product change                                                       | Blocking `mobile-coverage` job; floors in `mobile/jest.config.cjs`             | Raise the floors as the product grows                                         |
| Android native compile   | Pull requests and `main` changes under `mobile/` or its workflow/config             | Blocking clean Expo prebuild plus Gradle `assembleDebug`                       | Keep native plugins and configuration compilable                              |
| Maestro critical paths   | Locally during development; in CI when services and product journeys are configured | Generated Android and iOS emulator targets plus a configurable native workflow | Extend the baseline flow with stable, focused journeys and any extra fixtures |
| iOS Simulator compile    | Weekly schedule or manual dispatch                                                  | Configurable `macos-15` job; not a Linux PR gate                               | Fund/enable the macOS runner and investigate failures                         |
| VoiceOver and TalkBack   | Before a release claims native accessibility                                        | Protocol and result format, not automation                                     | Record physical device, OS, build, operator, and findings                     |

The generated `.github/workflows/ci.yml` keeps source checks blocking. Which
jobs it contains follows the capabilities the product was generated with:
`backend`, `backend-msrv`, `api-drift`, and `docker-build` for a backend;
`web`, `web-coverage`, and the `e2e-web` browser matrix for a web app;
`mobile` and `mobile-coverage` for a mobile app; `observability-lint` always.
The
generated `.github/workflows/native.yml` is path-filtered, cancels superseded
runs, caches pnpm and Gradle downloads, uploads failure diagnostics, and makes
Android compilation blocking whenever its relevant trigger fires. Its weekly
iOS job compiles on a real macOS runner. Maestro runs in that workflow only
when manual input or `RUN_MAESTRO_NATIVE_GATE=true` requests it. Products with
server-dependent flows must start their test services before enabling that CI
step.

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

## Generated local QA targets

Every generated mobile product has local release-build targets in
`mobile/Makefile`. Combined backend and mobile products also expose them from
the repository root:

| Target                                      | Result                                                                                                                     |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `qa-android`                                | Starts isolated services and a dedicated Android AVD, builds and installs the release APK, then leaves it open             |
| `qa-ios`                                    | On macOS, starts isolated services and a dedicated iOS Simulator, builds and installs the release app, then leaves it open |
| `e2e-android-live`, `e2e-ios-live`          | Runs `mobile/.maestro/` against the open device                                                                            |
| `e2e-android`, `e2e-ios`                    | Starts the environment, runs Maestro, and cleans up                                                                        |
| `qa-android-down`, `qa-ios-down`, `qa-down` | Stops devices started by the targets and removes disposable service volumes                                                |

Mobile-only products run the same commands with `make -C mobile`. The Android
compatibility aliases are `qa-setup`, `e2e-mobile`, and `e2e-mobile-live`.
`BAUKIT_QA_SKIP_BUILD=1` reuses the last platform build during an exploratory
session. Do not use it after JavaScript, native dependency, or app configuration
changes.

The release builds embed their JavaScript and do not need Metro. A QA-only Expo
config plugin permits local HTTP in the Android release manifest and iOS
transport settings because the isolated API and Keycloak use localhost.
Production builds do not load that plugin. The compose override uses its own
project name, ports, and volumes so cleanup cannot remove normal development
data.

The generated Maestro flow covers startup, local OIDC sign-in when selected,
the main screen, and a persisted preference. It is a baseline, not a claim that
the product's workflows are covered. Add product journeys under
`mobile/.maestro/` and keep manual checks for keyboard behavior, safe areas,
orientation, permissions, deep links, and accessibility services.

## Smoke and release evidence

Native compile proves linkage, not behavior. Product smoke coverage should
open at least one custom-scheme deep link from a terminated app and verify the
expected valid, invalid, and unauthenticated route outcomes. With the keyboard
open, verify focused controls remain visible, dismissal does not lose input,
and content/overlays respect top and bottom safe-area insets in portrait and
landscape where supported.

Route names, extra OAuth callbacks, screen wrappers, test accounts, backend
topology, and product journeys stay product-owned. Before release, combine
automated results with the physical VoiceOver/TalkBack protocol in the
[accessibility contract](./accessibility-contract.md).
