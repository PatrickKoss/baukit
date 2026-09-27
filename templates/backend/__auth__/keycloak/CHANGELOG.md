# Changelog

## [Unreleased]

- Add `js/theme-preferences.js` to `baukit-accessible`. It reads an `ap1` appearance hint from the OAuth `state`, or from `client_data` after a form post, pins light or dark mode, and exposes the app's colors as `--baukit-auth-primary`, `--baukit-auth-secondary`, and `--baukit-auth-on-primary`.
- Add the script-only `baukit-accessible` child theme, a neutral child fixture, generated realm selection, a read-only Compose mount, and pinned Keycloak browser tests.
- Add an explicit development-realm policy declaration and policy validator.
- Add an idempotent Keycloak reconciler for retained development volumes.
- Change the fresh development user password to `development-password` so it meets the generated minimum length. Existing volumes keep their current password unless reset explicitly.
