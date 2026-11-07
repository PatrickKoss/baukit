# Changelog

## Unreleased

## [0.19.0] - 2026-11-07

- Lints with `@baukit/eslint-config` and builds the web export's CSP with `@baukit/csp`.

## [0.18.3] - 2026-11-07

- The app config cases moved from `scripts/sign-in-config.test.mjs` to `src/app-config.test.ts` under Jest. The node test keeps the Metro cases. `check-production-bundle.mjs` passes the caught error as `cause`.

## [0.18.2] - 2026-11-06

- Metro refuses a `BAUKIT_QA_BUILD` set in a `mobile/.env*` file. The app config passes the local realm only to development and QA builds and `apiBaseUrl` only when `EXPO_PUBLIC_API_URL` is set. `check:bundle` exports with `--clear` and `mobile/.env` loaded, and scans the Expo web export and copied public files.

## [0.18.1] - 2026-11-05

- Generated mobile apps pin Expo 57.0.32, expo-router 57.0.30, expo-constants 57.0.23 and @expo/metro-runtime 57.0.18, the SDK 57 patch releases, and list those releases in `minimumReleaseAgeExclude` until they are a day old.

## [0.18.0] - 2026-11-05

## [0.17.1] - 2026-11-03

## [0.17.0] - 2026-11-02

## [0.16.0] - 2026-11-02

## [0.15.0] - 2026-11-02

## [0.14.1] - 2026-11-01

## [0.14.0] - 2026-11-01

## [0.13.0] - 2026-10-31

## [0.12.2] - 2026-10-31

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

- Generated mobile apps pin Expo 57.0.31 and expo-router 57.0.29 to match Expo dependency checks.

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

- Generated mobile apps pin Expo 57.0.30, expo-auth-session 57.0.16, expo-constants 57.0.22,
  expo-linking 57.0.13, expo-router 57.0.28 and @expo/metro-runtime 57.0.17.
- Document and test explicit cleanup outcomes for erased profiles so successful erasure does not imply a cleanup failure.

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

- Move shipped notes out of Unreleased into their release sections.

## [0.9.0] - 2026-10-07

- Select the {{ context.auth_provider }} adapter through the shared authentication contract.

## [0.8.0] - 2026-10-07

- Update Expo SDK 57 patches to match Expo compatibility checks.

## [0.7.3] - 2026-10-05

- Use dot-separated OIDC keys directly. Remove the colon-to-dot storage rewrite.

## [0.7.2] - 2026-10-05

- Use a valid SecureStore key for the local-data registry so identity bootstrap can open its partition.
- Wait for polling and accessibility effects in profile deletion tests.

## [0.7.0] - tag v0.7.0
Tagger: Patrick Koss <pati.koss@gmx.de>

baukit 0.7.0
2026-10-04

- Clear committed erasure keys and poll pending deletions automatically. Distinguish unsent requests and expired status tokens.
- Add confirmed profile and sign-in account deletion, durable retries, local account cleanup, and accessible pending and error states in English and German.
