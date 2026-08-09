# Releasing baukit

Baukit uses one private release train for every Rust crate, every
`@baukit/*` package, and every template. A train has one semantic version and
one immutable Git tag: `baukit-vX.Y.Z`. Components do not receive independent
tags. The bare `X.Y.Z` is also the template version written to
`templates/VERSION` and recorded in generated products' `baukit.toml`.

The project remains on `0.x` until it has survived the adoption threshold in
the platform conventions. During `0.x`, a breaking change requires a minor
bump; compatible changes normally use a patch bump.

## Tool ownership

- `rust/release-plz.toml` defines the Rust version group, per-crate changelogs,
  private git-only mode, and the single `baukit-v{{ version }}` tag emitter.
  Every crate also has `publish = false`, so a train cannot reach crates.io.
- Changesets records TypeScript changes. Its fixed group advances all eight
  packages together, versions private packages, creates package changelogs,
  and emits no package tags. No workflow runs `changeset publish`.
- `scripts/release-train.sh` is the cross-ecosystem coordinator. A standalone
  `release-plz release-pr` cannot atomically include Changesets and the
  template marker, and release-plz's git-only tagged-worktree handling expects
  the Cargo manifest at the Git root and registry-resolvable packaged path
  dependencies; neither is true of this private monorepo. The coordinator
  therefore prepares one `release-plz-*` PR without adding a duplicate root
  Cargo workspace or a fake private registry.

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
   version=0.3.0
   scripts/check-version-coherence.py --tag "baukit-v${version}"
   git tag -a "baukit-v${version}" -m "baukit ${version}"
   git push origin "baukit-v${version}"
   ```

The workflow automates coherent version changes, lockfile/changelog updates,
Rust/TypeScript/example verification, release-PR creation or update, tag
validation, release-note aggregation, and the GitHub release. Selecting the
bump, updating/reviewing the compatibility matrix, merging the PR, and pushing
the post-merge tag remain deliberate maintainer actions.

## Consume a private train

Rust products pin the unified tag; Cargo locates the named crate inside the
repository:

```toml
baukit-runtime = { git = "ssh://git@github.com/patrickkoss/baukit.git", tag = "baukit-v0.3.0" }
```

pnpm products pin that same tag and the package subdirectory. Quote the value
because `&` is part of pnpm's git selector:

```json
{
  "dependencies": {
    "@baukit/api-runtime": "git+ssh://git@github.com/patrickkoss/baukit.git#baukit-v0.3.0&path:typescript/packages/api-runtime"
  }
}
```

Pin every Baukit dependency to the same tag. Renovate may propose a newer
`baukit-v*` tag, but product CI must verify the generated fixture and the
compatibility matrix before merging the upgrade.

## Public-registry transition

Until the go-public decision, do not run `cargo publish`, `pnpm publish`, or
`changeset publish`. Going public changes dependency sources and release
credentials; it does not change crate names, package import names, or the
single-train compatibility rule.
