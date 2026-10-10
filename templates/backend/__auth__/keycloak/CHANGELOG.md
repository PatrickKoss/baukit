# Changelog

## [Unreleased]

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

- Reconcile MCP protocol mappers and callback URIs exactly. Remove stale audiences and preserve callback order returned by Keycloak.

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

- Move shipped notes out of Unreleased into their release sections.

## [0.7.3] - 2026-10-05

- Reconcile confidential backend clients without browser redirects. Keep creation secrets and preserve rotated secrets on existing clients.

## [0.7.1] - 2026-10-04

- Allow development reconciliation of `verifyEmail` and `resetPasswordAllowed`.

## [0.7.0] - tag v0.7.0
Tagger: Patrick Koss <pati.koss@gmx.de>

baukit 0.7.0
2026-10-04

- Grant and reconcile manage-users for backend account deletion.

## [0.5.0] - tag v0.5.0
Tagger: Patrick Koss <pati.koss@gmx.de>

baukit 0.5.0
2026-09-28

- Add `js/theme-preferences.js` to `baukit-accessible`. It reads an `ap1` appearance hint from the OAuth `state`, or from `client_data` after a form post, pins light or dark mode, and exposes the app's colors as `--baukit-auth-primary`, `--baukit-auth-secondary`, and `--baukit-auth-on-primary`.

## [0.3.0] - tag v0.3.0
Tagger: Patrick Koss <pati.koss@gmx.de>

baukit 0.3.0
2026-09-04

- Add the script-only `baukit-accessible` child theme, a neutral child fixture, generated realm selection, a read-only Compose mount, and pinned Keycloak browser tests.
- Add an explicit development-realm policy declaration and policy validator.
- Add an idempotent Keycloak reconciler for retained development volumes.
- Change the fresh development user password to `development-password` so it meets the generated minimum length. Existing volumes keep their current password unless reset explicitly.
