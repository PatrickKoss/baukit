# Changesets

Add a changeset with `pnpm changeset` for each user-visible TypeScript change.
The fixed package group advances every `@baukit/*` package together. The release
train consumes pending changesets with `pnpm version-packages`; it never runs
`changeset publish` while the repository is private.
