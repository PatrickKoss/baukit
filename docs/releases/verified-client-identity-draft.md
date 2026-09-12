# Verified client identity release notes

Prepared 2026-09-12 for the coordinated v0.4.0 release train. The source change
starts from `d55de50dcc8f46d15b45aed34e0f2335a65c1a4c`.

## Release notes

`baukit-auth` can expose a provider's verified OAuth client identity through
`Principal::client_id()`. Products opt in with
`PrincipalClaimMapping::client_id_claim`, for example `client_id_claim("azp")`
for Keycloak. No provider claim is mapped by default.

Signature and registered-claim validation still run before normalization.
A configured nonempty string is preserved exactly. Missing or null claims
produce `None`; other JSON types and empty strings fail authentication with
`InvalidPrincipalContext`. Internal and personal access-token principals have
no client ID. Organization and tenant mappings are unchanged.

Products must enforce their own client allowlist and reject missing identity
where it is required. An OAuth client ID does not prove that a public client is
an unmodified first-party application.

## Consumer migration

Existing applications need no configuration change. Applications that need
client-specific authorization should configure the provider claim explicitly,
then check the verified principal. Do not decode the bearer token again or put
client identity into an organization or tenant field.

Eigenruhe needs this field to deny animation playback grants to CLI/MCP and
personal access-token callers. Its dependency update and policy tests follow
the coordinated release. Its normal build must not depend on an unpublished
registry version or a developer's local source path.

## Release checklist

- [x] Add optional mapping/accessor without exposing arbitrary JWT claims.
- [x] Pass five signed-token regression tests, 35 auth unit tests and seven
  auth documentation tests.
- [x] Complete independent security and compatibility review. The reviewer found
  one missing mapped-client unknown-key test; that rejection case now passes.
- [x] Pass Rust workspace formatting, Clippy, tests and Rust 1.95 compiler gate.
- [x] Validate a generated backend and the Eigenruhe consumer with the local
  change without changing their published dependency pins.
- [x] Verify an explicitly OIDC-enabled generated backend as well.
- [x] Finish the repository's release CI and version-coherence checks.
- [x] Review the proposed version and release notes with the maintainer.
- [x] On a clean, reviewed worktree, run the coordinated release-train workflow
  for Rust, TypeScript, CLI, templates and charts. Do not bump only baukit-auth.
- [x] Obtain separate authorization to push and tag the release.
- [ ] Publish the Rust and TypeScript packages from the tagged commit.
- [ ] After publication, update Eigenruhe's release-train pins and lockfiles,
  configure the verified claim, and run its full gates.

## Verification record

The complete `make ci` gate passed with the combined authentication and mobile
QA changes. Cargo deny reported clean advisories and licenses. Rust 1.95 passed
the MSRV check. The full Docker-backed ignored Rust test suite and all six
generated fixture variants passed.

The native Android compile gate passed. Expo SQLite completed 23 checks on an
Android emulator, and the generated Android release build passed its Maestro
flow. Browser Dexie conformance also passed. The iOS simulator gate was not run
on the Linux release host because it requires Xcode on macOS.

The 0.4.0 package tree passed the npm dry run for all 18 packages. Package
publication remains manual and must use the tagged commit.
