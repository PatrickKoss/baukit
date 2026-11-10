# Changelog

## Unreleased

## [0.20.0] - 2026-11-10

## [0.19.2] - 2026-11-08

## [0.19.1] - 2026-11-08

## [0.19.0] - 2026-11-07

- Breaking, security: sign-in goes through the backend browser session (ADR 0006) with `BrowserSessionClient` from `@baukit/auth-web`. The refresh token stays server side behind a `__Host-` HttpOnly cookie, the access token lives in memory only, and tabs hand a refresh over to each other instead of racing it.

## [0.18.3] - 2026-11-07

- The production sign-in adapter reads its provider key with static `import.meta.env` access. `check:bundle` builds with a random `VITE_BUNDLE_CHECK_CANARY` value and fails when the bundle carries it.

## [0.18.2] - 2026-11-06

## [0.18.1] - 2026-11-05

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

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

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

- Select the {{ context.auth_provider }} SDK through the shared authentication contract.

## [0.7.2] - 2026-10-05

- Wait for automatic polling before asserting calls in profile deletion tests.

## [0.7.0] - tag v0.7.0
Tagger: Patrick Koss <pati.koss@gmx.de>

baukit 0.7.0
2026-10-04

- Clear committed erasure keys and poll pending deletions automatically. Distinguish unsent requests and expired status tokens.
- Add confirmed profile and sign-in account deletion, durable retries, local account cleanup, and accessible pending and error states in English and German.
