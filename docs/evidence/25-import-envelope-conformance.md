# Evidence 25: import-envelope conformance

## Source product files

- `/home/patrick/projects/eigenruhe/mobile/src/features/data-transfer/json.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/data-transfer/import-service.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/data-transfer/import-contract.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/data-transfer/import-service.test.ts`
- `/home/patrick/projects/tiefgang/mobile/src/integrations/files/json.ts`
- `/home/patrick/projects/tiefgang/mobile/src/integrations/toggl/import.ts`
- `/home/patrick/projects/tiefgang/mobile/src/integrations/toggl/import.test.ts`

## Observed failure or repeated glue

Eigenruhe validates a versioned JSON envelope, allowlists tables and fields, rejects ownership and
sync metadata, caps source bytes and rows, previews conflicts, and commits through one local
transaction. Tiefgang validates its JSON export but does not write it. Its Toggl CSV path previews
and deduplicates rows, then writes each session separately, so a later failure can leave a partial
import and sync metadata can become visible between rows.

Neither product has a Rust import consumer. The only matching backend reference is Eigenruhe's
`PracticeSource::Import` enum value. It does not decode or commit import envelopes, so this item does
not add a `baukit-test` helper.

## Baukit owner

`@baukit/data-contracts` owns bounded preparation, top-level field filtering, one-transaction
commit orchestration, post-commit notification order, and the optional Vitest conformance suite.
Products own their envelope and data rules.

## Public types and errors

Production exports include `ImportEnvelopeSource`, `ImportEnvelopeLimits`,
`DecodedImportEnvelope`, `DecodedImportEnvelopeRow`, `ImportEnvelopePreview`,
`ImportEnvelopeTransactionAdapter`, `PrepareImportEnvelopeOptions`,
`CommitImportEnvelopeOptions`, `prepareImportEnvelope`, and `commitImportEnvelope`.
`ImportEnvelopeError.code` is one of `source_too_large`, `too_many_rows`,
`collection_not_allowed`, `field_not_allowed`, `string_too_large`, or `invalid_row_value`.
Decoder, row decoder, planner, transaction, and post-commit errors pass through unchanged.

The `/vitest` entry exports `describeImportEnvelopeContract` and its fixture, harness, observation,
and options types.

## Product-owned inputs

Products supply source and schema versions, the envelope and row decoders, collection and field
allowlists, source, row, and string limits, conflict rules, transaction implementation, provenance,
deletion order, sync trigger, and user-facing messages.

## Cases

- Concurrency: preparation has no write adapter. Products recheck conflicts inside the commit
  transaction and serialize commits in their storage adapter.
- Failure: fixtures cover unknown fields, duplicate IDs, tombstones, ownership and revision fields,
  oversized strings, excess rows, mixed versions, and a failure after the first of two writes.
- Privacy: shared errors contain stable codes, locations, and numeric limits but no source values.
  Product decoders must keep source text out of their errors and logs.
- Cleanup: a failed transaction discards staged rows. Products release selected file contents and
  prepared plans when import UI closes or identity changes.

## Supported runtimes

The production helper targets ES2022 and uses no browser, Node, React, or database dependency. It
accepts strings and `Uint8Array`, so browsers, Node 24 or newer, and compatible React Native engines
can supply their normal file reader. The conformance entry requires Vitest in development only.

## Product adoption change

Eigenruhe can replace the safety orchestration and shared contract cases in
`mobile/src/features/data-transfer/import-service.ts` and `import-contract.test.ts`, while retaining
its JSON schema and repository mapping. Tiefgang can put the write loop in
`mobile/src/integrations/toggl/import.ts` behind one repository transaction and replace its atomicity
tests with the shared suite. Neither complete product file becomes deletable because both files
also contain product decoding and mapping.

## Implementation report

### 1. Summary

Added a two-step import helper to `@baukit/data-contracts`. Preparation checks source bytes before
decoding, enforces row and recursive string limits, rejects collections or fields outside the
product allowlist, decodes rows, and calls a preview planner without a write adapter. Commit runs
the complete plan through one transaction callback and invokes the product's state or sync callback
only after the transaction resolves.

The fixture-backed Vitest suite covers every case named in item 25 and can run against product
adapters. Package self-import tests cover the root, `./import-envelope`, and `./vitest` exports while
checking that previous root and Vitest exports remain available. No Rust helper was added because
neither audited product has a Rust envelope importer. This follows the plan's conditional Rust
target; there were no plan deviations.

### 2. Files added or changed

- `docs/evidence/25-import-envelope-conformance.md`
- `docs/platform/local-data-ownership-contract.md`
- `fixtures/import-envelope/import-envelope-v1.json`
- `typescript/.changeset/bounded-import-envelope.md`
- `typescript/packages/data-contracts/README.md`
- `typescript/packages/data-contracts/package.json`
- `typescript/packages/data-contracts/src/import-envelope.test.ts`
- `typescript/packages/data-contracts/src/import-envelope.ts`
- `typescript/packages/data-contracts/src/import-envelope.vitest.ts`
- `typescript/packages/data-contracts/src/index.ts`
- `typescript/packages/data-contracts/src/vitest.ts`

### 3. Verification

- `corepack pnpm --dir typescript install --frozen-lockfile`: passed for 19 workspace projects;
  lockfile unchanged.
- `corepack pnpm --dir typescript --filter @baukit/data-contracts run build`: passed. The first
  development run failed on one readonly-array test type, which was fixed before the final run.
- `corepack pnpm --dir typescript --filter @baukit/data-contracts run lint`: passed with zero
  warnings. An earlier development run found two new lint errors, both fixed before the final run.
- `corepack pnpm --dir typescript --filter @baukit/data-contracts run test`: passed, 5 files and 98
  tests. Earlier passing runs had 94 tests before the final four error-detail cases were added.
- `corepack pnpm --dir typescript --filter @baukit/data-contracts run format:check`: passed. Its
  first run found one new test-file formatting difference, which was formatted before the rerun.
- `corepack pnpm --dir typescript run check`: passed in both runs, 72 of 72 tasks each time.
- `corepack pnpm --dir typescript/packages/data-contracts pack --pack-destination <temporary-dir>`
  plus tar inspection: passed. The archive contains the import module and declarations, root and
  Vitest entries, and the `./import-envelope` manifest export.
- `corepack pnpm --dir typescript exec prettier --check ...`: passed for the owned README, platform
  contract, evidence, fixture, changeset, source, tests, and package manifest.
- `git diff --check -- <owned tracked files>`: passed.
- `make expo-sqlite-conformance`: passed. Android build completed 213 tasks and the real Expo SQLite
  app reported `BAUKIT_SQLITE_CONFORMANCE_PASS {"passed":23}`.
- `make ci`: the first run failed on concurrent CLI formatting. After that owner formatted the
  files, `cargo fmt --manifest-path cli/Cargo.toml --all --check` passed and a second run reached the
  CLI tests. It failed there with 29 passed and 7 failed snapshot or generated-file cases. Before
  the failure, TypeScript formatting and lint passed 18 of 18 tasks, Rust format, clippy, tests, and
  check passed, TypeScript check passed 72 of 72 tasks, and the browser suite passed 2 files and 26
  tests across Chromium and WebKit. A final run against the updated shared tree passed dependency
  installation and TypeScript formatting for 18 of 18 packages, then failed Rust formatting in the
  concurrent `baukit-test` inbox and webhook files.
- Docker-gated suites: none apply and none ran because this item changed no Rust crate or
  Docker-backed adapter.

### 4. Failures observed in other agents' areas

The second `make ci` run failed seven concurrent CLI generator cases:
`doctor_requires_generated_environment_and_strict_markdown_scripts`,
`generated_markdown_link_check_fails_for_a_committed_missing_target`,
`generated_migration_guard_ports_failure_cases`,
`mcp_generation_matches_golden_tree_and_records_personal_token_auth`,
`oidc_generation_is_deterministic_and_records_the_optional_capability`,
`quality_flag_generates_the_strict_profile`, and
`strict_generation_is_capability_driven_and_matches_golden_tree`. I did not edit those paths.
The final run failed `cargo fmt --manifest-path rust/Cargo.toml --all --check` on concurrent changes
in `rust/crates/baukit-test/src/inbox.rs` and `webhook.rs`.

### 5. Leftovers and open questions

- Rerun `make ci` after the Rust owner formats its new files and the CLI owner verifies the
  generator snapshots.
- Product adoption remains outstanding because product repositories are read-only in this batch.
- The release must consume the changeset before either product can pin the public package.

### 6. Product adoption note

No complete product implementation file becomes deletable. Eigenruhe can delete its duplicated
safety orchestration and replace the cases in
`mobile/src/features/data-transfer/import-contract.test.ts` with shared suite registration.
Tiefgang can delete the per-row commit loop in `mobile/src/integrations/toggl/import.ts` after its
repository exposes one transaction, then replace the matching atomicity cases in `import.test.ts`.
