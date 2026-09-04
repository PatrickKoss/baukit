# Changelog

## [Unreleased]

- Add an explicit development-realm policy declaration and policy validator.
- Add an idempotent Keycloak reconciler for retained development volumes.
- Change the fresh development user password to `development-password` so it meets the generated minimum length. Existing volumes keep their current password unless reset explicitly.
