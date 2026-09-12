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
- [ ] On a clean, reviewed worktree, run the coordinated release-train workflow
  for Rust, TypeScript, CLI, templates and charts. Do not bump only baukit-auth.
- [x] Obtain separate authorization to push and tag the release.
- [ ] Publish the Rust and TypeScript packages from the tagged commit.
- [ ] After publication, update Eigenruhe's release-train pins and lockfiles,
  configure the verified claim, and run its full gates.

The release-train script requires a clean worktree. This draft does not bypass
that requirement or imply that the unchecked release gates have passed.

## Verification record

Rust workspace tests and MSRV checks passed. Logs are
`/tmp/baukit-client-identity-workspace-tests.log` and
`/tmp/baukit-client-identity-msrv.log`. The final focused signed-token suite also
passed after the unknown-key case was added.

The full `make ci` run reached CLI tests but failed four generated-tree snapshot
checks. During the run, separate mobile QA template edits appeared outside this
auth change. Their ownership is being confirmed. No snapshots were regenerated
and those edits were preserved. Full CI is not green. Its log is
`/tmp/baukit-client-identity-ci.log`.

A generated backend passed formatting, Clippy, its normal tests and explicit
OpenAPI drift. Evidence is under `/tmp/baukit-auth-consumer.1y29070B`. Its normal
gate leaves one PostgreSQL test ignored. Local auth resolves through its test
dependency, so an OIDC-enabled fixture was checked separately.

The OIDC-enabled fixture at `/tmp/baukit-oidc-consumer.249eWL1h/oidc-fixture`
also passes formatting, Clippy, 12 normal tests and explicit OpenAPI drift.
Its API and bin crates directly use the local auth crate for production OIDC
composition. Three auth conformance tests cover protected-route identity and
auth-before-rate-limit ordering. One unrelated Docker PostgreSQL adapter test
remains ignored in the normal fixture command. This completes the additional
OIDC consumer check.

The Eigenruhe backend workspace tests passed against all local Baukit crates
in `/tmp/eigenruhe-baukit-client-consumer-XvWPaO`. CLI-only Cargo patch options
selected the local crates in that disposable source copy. The first two runs
found missing copied content fixtures; after those files were copied, all
normal workspace tests passed. Docker-backed ignored tests were not selected
in this consumer run. Eigenruhe's actual manifest and lockfile were untouched.
