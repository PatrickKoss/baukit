# Changelog

## Unreleased

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
