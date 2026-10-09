# Changelog

## Unreleased

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

- Select the {{ context.auth_provider }} SDK through the shared authentication contract.

## [0.7.2] - 2026-10-05

- Wait for automatic polling before asserting calls in profile deletion tests.

## [0.7.0] - tag v0.7.0
Tagger: Patrick Koss <pati.koss@gmx.de>

baukit 0.7.0
2026-10-04

- Clear committed erasure keys and poll pending deletions automatically. Distinguish unsent requests and expired status tokens.
- Add confirmed profile and sign-in account deletion, durable retries, local account cleanup, and accessible pending and error states in English and German.
