# Changelog

## Unreleased

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
