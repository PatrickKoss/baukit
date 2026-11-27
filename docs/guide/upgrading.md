# Upgrade a product to a new baukit release

baukit ships crates, npm packages, templates and the Helm chart as one release train with one
version. This page moves a product from one train to the next. At the end every pin names the
new version, the template-owned files carry the release's changes, and the product's checks pass.

There is no `baukit upgrade` command. You bump the pins, apply the changelog, and let
`baukit doctor` tell you what still disagrees. [ADR 0008](../adr/0008-release-train-and-support-policy.md)
covers the release train and the support window.

## Before you start

1. Start from a clean worktree and note the capabilities, profile and `[dependencies.baukit]`
   source in `baukit.toml`.
2. Read every `templates/common/CHANGELOG.md` entry after your `template_version`, and the
   target's section of `docs/platform/compatibility-matrix.md`.
3. Install the CLI from the target tag:

   ```sh
   cargo install --git https://github.com/PatrickKoss/baukit --tag vX.Y.Z --locked baukit-cli
   ```

## Bump the pins

All components move together. Mixed versions fail doctor.

1. Set every `baukit-*` crate in `backend/Cargo.toml`, and in `backend/fuzz/Cargo.toml` when it
   exists, to the target version.
2. Set every `@baukit/*` dependency in `web/package.json` and `mobile/package.json` to the exact
   target version.
3. In `baukit.toml`, set `template_version` and `[dependencies.baukit] version` to the target.
   A product still on `source = "git"` moves to `source = "registry"`.
4. Keep your `[quality]` thresholds, critical paths and `openapi.consumers`.

## Apply template changes

Apply each changelog migration to the files it names. To pick up files the new template adds,
re-render into the product as [scaffold a product](scaffolding.md#add-files-to-an-existing-product)
describes. It writes only missing files and lists the ones that differ in `baukit-conflicts.txt`.

Then refresh the generated pieces:

```sh
baukit check agent-docs --fix
baukit check template-files --update
make lockfiles
```

`agent-docs --fix` rewrites the managed rules block in `CLAUDE.md`. `template-files --update`
records the new hashes. Keep unrelated dependency bumps out of this change.

## Inside the first day

pnpm refuses a package for a day after its publish when `minimumReleaseAge` is set, and that
covers the `@baukit/*` packages your pins pull in transitively. Print the exclusions:

```sh
baukit generate release-age-excludes
```

Paste each block into the `pnpm-workspace.yaml` it names, and delete it once the release is a day
old.

## Verify

```sh
baukit doctor
baukit check
make check
baukit generate openapi-client
cargo test --manifest-path backend/Cargo.toml --test openapi_drift
```

Run the web and mobile installs, builds, lint and tests for the apps you have. In a strict
product finish with the gate, pointing the migration guard at the commit before the upgrade:

```sh
BAUKIT_BASE_REVISION=<pre-upgrade-commit> sh scripts/quality-gate.sh
```

The guard then fails if the upgrade rewrote an applied migration instead of adding one.

## Reference

- `templates/common/CHANGELOG.md`
- [`docs/platform/compatibility-matrix.md`](../platform/compatibility-matrix.md)
- [`docs/migrations/README.md`](../migrations/README.md)
- [`agent-skills/skills/baukit-upgrade/SKILL.md`](../../agent-skills/skills/baukit-upgrade/SKILL.md)
- [The baukit CLI](cli.md)
- [ADR 0008: release train and support policy](../adr/0008-release-train-and-support-policy.md)
