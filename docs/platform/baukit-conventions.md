# Baukit conventions: naming, licensing, MSRV, releases, support

**Status:** Adopted.
**Home:** this repository.

## Naming

- The name is **baukit** (decided 2026-08-08; a nod to *Baukasten*, a modular construction kit). At decision time the npm package name, crates.io name, and GitHub organization/username were all verified free.
- Everything is private-first: repositories live under `github.com/patrickkoss` and nothing is published to public registries until the go-public decision (Phase 4 of the analysis). Optionally register the free reservations early as squatting protection (the `@baukit` npm organization, which also confirms scope availability, and the GitHub organization), but publish nothing under them.
- At the go-public decision: create/activate the GitHub and npm organizations, publish the crates.io names, check `baukit.dev`, and add license files.
- Rust crate names live in [`rust/crates/*/Cargo.toml`](../../rust/crates/). The [README crate table](../../README.md#rust) describes all eighteen, including `baukit-erasure`.
- npm package names live in [`typescript/packages/*/package.json`](../../typescript/packages/). The [README package table](../../README.md#typescript) describes all twenty-one, including `@baukit/navigation`.
- CLI binary: `baukit`.

## Wire names

JSON property names and path and query parameter names in HTTP APIs are lower camelCase: an ASCII
lowercase letter followed by ASCII letters and digits, such as `requestId` or `nextCursor`. Rust
DTOs get `#[serde(rename_all = "camelCase")]`. Error `details` keys name fields by the same wire
name.

These stay as they are:

- values, including error codes such as `validation_failed`, enum values, and event type names;
- header names;
- keys of data maps such as a `details` map of product counters;
- names a standard defines, such as OAuth 2.0 `access_token`, which the product lists as exemptions;
- `baukit-ops` operational responses.

`baukit_openapi::check_camel_case_names` walks a generated OpenAPI document and reports each
violation with its JSON pointer. Generated products run it in the `openapi_drift` test.

## Licensing

- Everything starts private and unlicensed (proprietary by default); the licenses below are added at the go-public decision, not before.
- Rust crates: dual `MIT OR Apache-2.0` once public.
- TypeScript packages, templates, Helm chart, dashboards, and agent skills: MIT once public.
- Repositories that stay private (`platform-infra`, product repos): proprietary, no license file.

## MSRV and toolchains

- MSRV: current stable minus two minor versions at the time of the first release (verify against `rustup` at kickoff); recorded as `rust-version` in every crate and enforced by a dedicated CI job.
- MSRV bumps are minor version changes pre-1.0 and are called out in release notes.
- Node: current LTS. pnpm and Turbo versions are pinned via the `packageManager` field.

## Versioning and releases

- Everything stays 0.x until the fourth application (the architecture health platform) has survived two foundation upgrades. Pre-1.0, breaking changes bump the minor version.
- Rust: `release-plz` for versioning, changelogs, and release tags. Products consume the crates from crates.io pinned to an exact version; a git dependency on a release tag stays available for unreleased work.
- Third-party Rust dependencies use caret requirements with tested minimum versions. A bare version such as `"1.53.1"` means `^1.53.1`; `"0.9.0"` permits compatible 0.9 patches. Products own their lockfiles and can update these dependencies within the declared ranges without a Baukit release. Baukit commits its lockfiles to keep CI reproducible and verifies both direct minimums and the newest compatible resolution when changing requirements. Exact third-party pins need a documented semver incompatibility, a comment beside the requirement, and an exception in `scripts/test_rust_dependency_requirements.py`. Internal `baukit-*` requirements stay exact because the crates release together.
- TypeScript: Changesets for versioning and changelogs; no npm registry while private. Products consume packages as pnpm git dependencies pinned to release tags, resolving subdirectory packages from the baukit monorepo, with a `prepare` script building on install (or committed dist output). Going public is only a dependency-source switch from git tag to npmjs version; the `@baukit/*` import names never change. Renovate updates the pinned tags.
- Templates: versioned with the platform release train; `baukit.toml` records the template version used at generation time.
- One release train: a platform release bumps crates, packages, and templates together and updates the [compatibility matrix](./compatibility-matrix.md). There are no independent template releases.

## Support policy

- The latest minor of the latest release train is supported; no backports pre-1.0.
- Every published package carries a maturity label: `experimental`, `stable-core`, or `internal`. Only `stable-core` implies upgrade care for external users.
- Security reports go to a private contact defined in `SECURITY.md`; fixes land in the current train.
