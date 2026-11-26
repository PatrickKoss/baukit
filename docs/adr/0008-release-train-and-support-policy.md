# ADR 0008: All units release together on one train, products pin registry versions, and the license is MIT

## Status
Accepted, 2026-11-27. The support policy section is Proposed.

## Context

Baukit ships Rust crates, npm packages, CLI templates and a Helm chart. Products generated from the
templates depend on many of them at once. Independent versions would force every product to solve
a compatibility matrix by hand, and a template could reference a crate version that never shipped
with it.

Baukit is pre-1.0 and has one maintainer. A support promise wider than the latest release would be
a promise nobody keeps.

## Decision

- One version covers every crate, package, template and the chart. `templates/VERSION` holds it.
  `rust/release-plz.toml` puts all crates in `version_group = "baukit-release-train"`, and
  Changesets uses one `fixed` group. `scripts/release-train.sh` cuts a release, and one tag
  `vX.Y.Z` marks it. [docs/releasing.md](../releasing.md) holds the steps.
- During 0.x a breaking change bumps the minor version and a compatible change bumps the patch.
- Generated products pin registry releases. `registry_dependency_context` in
  `cli/src/generate/dependencies.rs` writes `baukit-x = "X.Y.Z"` for Cargo, which is a caret
  requirement, the exact version for npm, and `source = "registry"` in `baukit.toml`.
  `--baukit-path` swaps in local paths for fixture gates.
- Every crate and package is MIT licensed.

### Support policy

Only the latest minor gets fixes. There are no backports before 1.0. Products upgrade to the newest
release to get a fix. Nothing in the repository states maturity labels per unit, and there is no
`SECURITY.md`, so this part is Proposed.

## Enforced by

- Gate step `version-coherence` runs `scripts/check-version-coherence.py`. It fails when a crate,
  package, template, the chart, a lockfile, the Changesets `fixed` group or the release-plz group
  disagrees with `templates/VERSION`, and when `rust/Cargo.toml` or a package does not declare MIT.
- Nothing enforces the support policy. A `SECURITY.md` with the supported range, checked by the
  same script against `templates/VERSION`, would.

## Consequences

A product upgrades by changing one version everywhere, and templates always match the crates they
name. A fix to one crate releases every unit, so most releases carry unchanged units. Products on an
older minor get no fixes.

## History

- 2026-08-25, commit `af6e2b4f` (baukit 0.1.0): one train from the start. Generated products then
  pinned a git tag with `source = "git"`.
- 2026-08-25, commit `798d1f49`: the license moved to MIT only.
- 2026-08-26, commit `3a7b75e1`: generated products switched from git tags to registry versions.
