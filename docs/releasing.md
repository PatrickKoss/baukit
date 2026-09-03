# Releasing baukit

Baukit uses one release train for every Rust crate, every
`@baukit/*` package, and every template. A train has one semantic version and
one immutable Git tag: `vX.Y.Z`. Components do not receive independent
tags. The bare `X.Y.Z` is also the template version written to
`templates/VERSION` and recorded in generated products' `baukit.toml`.

The project remains on `0.x` until it has survived the adoption threshold in
the platform conventions. During `0.x`, a breaking change requires a minor
bump; compatible changes normally use a patch bump.

## Tool ownership

- `rust/release-plz.toml` defines the Rust version group, per-crate changelogs,
  and the single `v{{ version }}` tag emitter. The sixteen library crates
  publish to crates.io; the `baukit` CLI keeps `publish = false`.
- Changesets records TypeScript changes. Its fixed group advances all seventeen
  packages together, creates package changelogs, and emits no package tags.
  Publishing to npm is a separate step (see below), not `changeset publish`.
- `scripts/release-train.sh` is the cross-ecosystem coordinator. A standalone
  `release-plz release-pr` cannot atomically include Changesets and the
  template marker, and release-plz's git-only tagged-worktree handling expects
  the Cargo manifest at the Git root and registry-resolvable packaged path
  dependencies; neither is true of this monorepo. The coordinator therefore
  prepares one `release-plz-*` PR without adding a duplicate root Cargo
  workspace or a fake private registry.

## Record changes

For a user-visible TypeScript change, run this from `typescript/` and commit
the resulting Markdown file with the change:

```sh
pnpm changeset
pnpm changeset status
```

Choose the bump appropriate to the change. The fixed group ensures that one
package change advances every `@baukit/*` package. Keep the affected Rust
crate's `[Unreleased]` changelog section current as part of the same change;
release-plz's changelog format and conventional commits remain the source
convention for those files.

## Cut a train

1. Start from current `main`. Confirm the tree is clean and all pending work is
   merged.
2. Check pending TypeScript notes with `pnpm --dir typescript changeset status`.
3. Run **Release train** in GitHub Actions and choose `patch` or `minor`.
   Equivalently, from a clean local checkout with dependencies installed, run
   `scripts/release-train.sh patch` (or `minor`) and open a PR from its changes.
4. On the generated release-PR branch, update
   `docs/platform/compatibility-matrix.md` to the dependency and toolchain
   versions actually tested by this train. This step is manual because the
   values describe evidence, not merely lockfile contents.
5. Review the generated package changelogs, the new sections in every crate
   changelog, the compatibility matrix, and CI. Merge the release PR.
6. From the merged `main`, verify and push the unified annotated tag:

   ```sh
   version=0.1.0
   scripts/check-version-coherence.py --tag "v${version}"
   git tag -a "v${version}" -m "baukit ${version}"
   git push origin "v${version}"
   ```

The workflow automates coherent version changes, lockfile/changelog updates,
Rust/TypeScript/example verification, release-PR creation or update, tag
validation, release-note aggregation, and the GitHub release. Selecting the
bump, updating/reviewing the compatibility matrix, merging the PR, and pushing
the post-merge tag remain deliberate maintainer actions.

## Consume a train from Git

Rust products normally depend on the published crates. To pin the Git tag
instead, Cargo locates the named crate inside the repository:

```toml
baukit-runtime = { git = "https://github.com/PatrickKoss/baukit.git", tag = "v0.1.0" }
```

TypeScript packages are on npm (see below), so pnpm products normally install
`@baukit/*` from the registry. To pin a Git tag instead, quote the value
because `&` is part of pnpm's git selector:

```json
{
  "dependencies": {
    "@baukit/api-runtime": "git+https://github.com/PatrickKoss/baukit.git#v0.1.0&path:typescript/packages/api-runtime"
  }
}
```

Pin every Baukit dependency to the same tag. Renovate may propose a newer
`v*` tag, but product CI must verify the generated fixture and the
compatibility matrix before merging the upgrade.

## Publish the TypeScript packages

The seventeen `@baukit/*` packages are published to npm under the `baukit`
organization scope, MIT licensed. Each one sets `publishConfig.access` to
`public`; `scripts/check-version-coherence.py` fails the train if a package
loses that setting or its licence.

From a clean checkout at the tagged commit:

```sh
corepack pnpm --dir typescript install --frozen-lockfile
corepack pnpm --dir typescript build
corepack pnpm --dir typescript publish -r --access public --dry-run
corepack pnpm --dir typescript publish -r --access public
```

pnpm walks the packages in dependency order and rewrites `workspace:*`
requirements to the published version. Only `dist/`, `README.md`, and
`LICENSE` ship in each tarball.

A published version is permanent: npm allows unpublishing only within 72 hours,
and never allows reusing the version number afterwards. Run the dry run first.

## Publish the Rust crates

The sixteen library crates are published to crates.io, MIT licensed. The
`baukit` CLI keeps `publish = false` and stays a Git-tag install.
`scripts/check-version-coherence.py` fails the train if a library crate regains
that flag or the workspace loses its licence.

Crates must go up in dependency order, because each one's internal
`version = "=X.Y.Z"` requirements must already resolve on the registry:

```text
baukit-core, baukit-events, baukit-openapi, baukit-sync, baukit-config,
baukit-runtime, baukit-telemetry, baukit-credential-vault, baukit-http,
baukit-jobs, baukit-ops, baukit-auth, baukit-integrations, baukit-push,
baukit-ratelimit, baukit-test
```

Several crates dev-depend on siblings for test fixtures, and those edges form a
cycle: `baukit-test` depends on `baukit-integrations`, which dev-depends on
`baukit-jobs`, which dev-depends back on `baukit-test`. Cargo resolves a
dev-dependency against the registry whenever it carries a version, so no publish
order can satisfy that. These dependencies are therefore declared path-only,
which makes `cargo package` strip them from the published manifest while local
`cargo test` still resolves them. `scripts/check-version-coherence.py` fails the
train if one regains a version or `workspace = true`.

crates.io rate-limits new crate names: an initial burst, then roughly one new
crate every ten minutes. A first release of all sixteen therefore cannot run
straight through. `scripts/publish-crates.sh` skips crates already on the
registry and waits out a 429 instead of failing the run:

```sh
cargo login
scripts/publish-crates.sh
```

Later trains publish new versions of existing crates, which fall under a much
higher limit, so they finish in one pass.

A dry run only works for crates whose internal dependencies are already
published; `cargo publish --dry-run` on a dependent crate fails with
"no matching package named ..." until its siblings are up. That is expected,
not a defect.

A crates.io release is permanent. Versions can be yanked but never deleted or
reused, and a yanked version still resolves for existing lockfiles.
