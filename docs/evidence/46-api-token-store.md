# PostgreSQL API token store with grants evidence

Plan item 10, "Add a PostgreSQL API token store with grants". Steps 1 to 3 are implemented in
`baukit-auth`. Step 4, product adoption, is deferred. The route credential policy layer and a
`CredentialVault` store were out of scope and appear below only as follow-up studies.

## Source revisions

- Baukit baseline `9cbdf55`.
- Eigenruhe `f74cebb`.
- Hebkit `841bf5d`.
- Leitbild `bd38b33`.
- Runtime Analyzer `d47bfd5`.
- Tiefgang `2d37a06`.
- Redemut `a782538`, read for issuer routing, audience, scope, and profile claims.
- Solo Leveling System `3461eaf`, read for profile claims only.
- Schlauzug `31d3f55`. Its uncommitted files are provisional evidence and only confirm the scope
  pattern.

## Step 1: five token schemas on six points

| Point | Eigenruhe | Hebkit | Leitbild | Runtime Analyzer | Tiefgang |
|---|---|---|---|---|---|
| Digest | SHA-256 `BYTEA UNIQUE` | SHA-256 `bytea UNIQUE` | SHA-256 `BYTEA UNIQUE` | SHA-256 hex `TEXT` | SHA-256 `BYTEA UNIQUE`, `octet_length = 32` |
| Prefix | `token_prefix` | `token_prefix` | `token_prefix` | `prefix` | `token_prefix`, at most 16 |
| Expiry | `expires_at` | `expires_at` | `expires_at` | `expires_at` | `expires_at` |
| Last used | `last_used_at`, unconditional overwrite | same | same | same | same, plus `last_seen_at` for one token kind |
| Ownership | `owner_id` FK `user_identities` cascade | `user_id` FK `users` cascade | `user_id` FK `user_identities` cascade | `tenant_id` plus `owner_id` or `cluster_id`, row-level security | `user_id` FK `users` cascade |
| Revocation | `revoked_at` | `revoked_at` | `revoked_at`, partial index | `revoked_at` | `revoked_at` |
| Grants | `scopes TEXT[]`, known-list CHECK | `scopes text[]` | `permissions TEXT[]`, known-list CHECK | `scopes JSONB` plus `role` | `scopes TEXT[]`, at most 16, per-kind CHECK |

Sources:

- Eigenruhe `backend/migrations/20260823000000_sync_foundation.sql:46-58` and
  `20260927000000_api_token_scopes.sql`, adapter `crates/eigenruhe-postgres/src/api_tokens.rs`,
  service `crates/eigenruhe-services/src/api_tokens.rs`.
- Hebkit `migrations/20260827100000_core.sql:64-79`, `20260927100000_api_token_scopes.sql`, and the
  name limit in `user_limits.sql:106`. Adapter
  `crates/hebkit-postgres/src/adapters/postgres/api_tokens.rs`.
- Leitbild `migrations/0011_create_api_tokens.sql`, `0013` (revoked index), and `0017`
  (`permissions`). Adapter `crates/leitbild-postgres/src/api_token.rs`, issue flow
  `crates/leitbild-api/src/api_token.rs:185-230`.
- Runtime Analyzer `migrations/0005_tenancy.sql:35-42`, `0006_clusters.sql:38-43`,
  `0018_identity_tokens.sql`, and the SECURITY DEFINER lookups in `0021_ingest_auth.sql`. Adapters
  `crates/finops-postgres/src/agent_tokens.rs` and `identity.rs:343` onward, scope filtering in
  `finops-api/src/auth/principal.rs:56-70`.
- Tiefgang `migrations/20260823000000_users_settings_sync.sql:49-64` and later limit migrations.
  Adapter `tiefgang-postgres/src/api_tokens.rs:52-81`, service
  `tiefgang-services/src/api_tokens.rs:69-106,173-250`.

### Decision: what Baukit owns and what the product joins

Four products already agree on everything except the owner column name and the grants column name.
Baukit takes the majority names and owns this table:

| Column | Type and constraint |
|---|---|
| `id` | `UUID PRIMARY KEY`, a v7 UUID from the service |
| `owner_id` | `UUID NOT NULL`, no foreign key in Baukit's migration |
| `name` | `TEXT`, 1 to 100 characters, trimmed by the service |
| `token_hash` | `BYTEA NOT NULL UNIQUE`, exactly 32 bytes |
| `token_prefix` | `TEXT NOT NULL`, non-empty |
| `grants` | `TEXT[] NOT NULL DEFAULT '{}'`, at most 64, no NULL elements |
| `created_at` | `TIMESTAMPTZ NOT NULL`, the service instant |
| `expires_at` | `TIMESTAMPTZ`, after `created_at` |
| `last_used_at` | `TIMESTAMPTZ` |
| `revoked_at` | `TIMESTAMPTZ` |

Plus the index `(owner_id, created_at DESC, id DESC)`, which serves both the list and the active
count.

The product joins or adds:

- The owner foreign key. Baukit cannot name `users` or `user_identities`, so the migration header
  shows the `ALTER TABLE ... REFERENCES <owner table> (id) ON DELETE CASCADE` a product adds.
- Known-grant CHECK constraints. Grant names are product vocabulary.
- A maximum token lifetime. Eigenruhe caps TTL in its service; that stays a product rule applied
  before `issue`.
- Tiefgang's `kind` column and its per-kind grant rules.
- Runtime Analyzer's tenant, role, rate-limit override, and row-level security.

Reasoning. The digest, the prefix, expiry, last use, and revocation have the same meaning in every
product, and `ApiTokenService` already decides all of them. Ownership is the only point where the
products differ in kind, and a foreign key added by the product covers all of the cascade cases
without Baukit knowing the owner table. Grants differ only in name and in which strings are valid,
so Baukit stores an opaque `BTreeSet<String>` and leaves validity to a product CHECK.

Runtime Analyzer does not fit. Its tokens are tenant-scoped under row-level security and reached
through SECURITY DEFINER functions, and its digests are hex text. It keeps its own adapter and can
still use `ApiTokenService` if it wants the service rules.

## Baukit owner

`rust/crates/baukit-auth`. The PostgreSQL adapter sits behind the optional `sqlx-postgres`
feature, following `baukit-sync` and `baukit-jobs`: optional `sqlx`, a reference migration in
`migrations/`, and a `*_MIGRATION_SQL` constant available without the feature. Products copy the
SQL; nothing migrates on startup.

## Public types and functions

Step 2, grants and the store:

- `NewApiToken::grants` and `NewApiToken::with_grants`.
- `ApiTokenRecord::grants` and `ApiToken::grants`.
- `ApiTokenError::InvalidGrants`, `MAX_API_TOKEN_GRANTS` (64), and `MAX_API_TOKEN_GRANT_LENGTH`
  (128 bytes).
- `Principal::grants() -> Option<&BTreeSet<String>>`.
- `POSTGRES_API_TOKENS_MIGRATION_SQL`.
- Behind `sqlx-postgres`: `PostgresApiTokenStore::new`,
  `PostgresApiTokenStore::with_active_token_limit`, `erase_owner_api_tokens`, and
  `purge_inactive_api_tokens`.

Step 3, verified claims and routing:

- `Principal::scopes`, `PrincipalClaimMapping::scope_claim`.
- `Principal::profile_claims`, `Principal::profile_claim`, `PrincipalClaimMapping::profile_claims`,
  and `ProfileClaim` with `as_str` and `as_bool`.
- `ClerkVerifier::with_audiences`, `ClerkVerifier::with_profile_claims`, `ClerkVerifier::issuer`,
  and the same three on `WorkOsVerifier`. `OidcVerifier::issuer`.
- `IssuerVerifier` with `From` for all three verifiers, and `MultiIssuerVerifier::from_verifiers`.

No new wire field exists. `Principal`, `ApiToken`, and `ProfileClaim` are not serialized by
Baukit, so the camelCase rule for new wire fields has nothing to apply to. A product that returns
grants in its token list response names that field itself.

## Contract as implemented

Grants:

- Each grant is an RFC 6749 scope token: 1 to 128 bytes from `%x21 / %x23-5B / %x5D-7E`. That
  excludes space, `"`, and `\`, so a grant set can be written as an OAuth `scope` string without
  escaping. A token carries at most 64 grants. An empty set is allowed.
- The service validates before the store is called. `ApiTokenError::InvalidGrants` carries no
  detail.
- The grants cap matches the database CHECK, so a validated set never fails the Baukit constraint.

Store:

- `create` writes grants in the same `INSERT` as the digest. Without a limit it is a single
  statement. With `with_active_token_limit`, one transaction takes
  `pg_advisory_xact_lock(hashtextextended('baukit_auth.api_tokens:' || owner_id, 0))`, counts
  unrevoked tokens not expired at the new token's `created_at`, and inserts. Over the limit it
  returns the configured `ApiTokenPolicyRejection` and writes nothing. The advisory lock needs no
  owner table, which is why Baukit uses it instead of the products' `SELECT ... FOR UPDATE` on the
  owner row.
- `find_by_hash` returns revoked and expired rows. The service decides.
- `touch_last_used` runs `UPDATE ... WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2)`.
  Two concurrent updates serialize on the row lock, and PostgreSQL re-evaluates the predicate
  against the committed row, so the latest instant wins.
- `revoke` matches `owner_id`, `id`, and `revoked_at IS NULL`. Someone else's token and an already
  revoked token both give `NotFound`.
- `list_for_owner` returns every token, revoked and expired included, ordered
  `created_at DESC, id DESC`. Products that hid revoked tokens filter on `revoked_at`.
- `erase_owner_api_tokens` takes any `PgExecutor`, so it runs inside the product's erasure
  transaction.
- `purge_inactive_api_tokens` deletes one batch of rows with `revoked_at` or `expires_at` before the
  cutoff, `FOR UPDATE SKIP LOCKED`, and returns the count.

Step 3:

- The `scope` claim is mapped by default for every token profile. The plan asks for verified scope
  on OIDC principals, and every product that reads scope reads the RFC 9068 `scope` string. A
  string splits on ASCII whitespace; an array of non-empty strings works for providers that emit
  `scp`-style arrays. Missing or null gives an empty set. Other shapes fail with
  `InvalidPrincipalContext`, the same rule as the organization, tenant, and client claims.
- Profile claims are opt-in by name. A selected claim that is a string or boolean is copied; any
  other type is left out without failing verification. This is looser than the scope rule on
  purpose. Redemut tolerates `preferred_username: 42` today, and failing a login over a malformed
  display name helps nobody. Omission is also the safe reading for `email_verified`: a string
  `"true"` is not a verified email.
- Clerk and WorkOS keep their documented behavior of not requiring `aud`. `with_audiences` turns on
  the same audience check the OIDC profile runs. The check now runs whenever the configuration has
  audiences, and OIDC configurations always have at least one.
- `MultiIssuerVerifier::from_verifiers` stores each verifier's inner OIDC verifier keyed by its
  normalized issuer. Provider checks stay attached to that verifier.
- Scopes and grants stay separate on `Principal`. An OIDC scope is what the identity provider
  granted a client; an API-token grant is what the product stored. Mapping one onto the other is
  the route credential policy study below.

## Cases

Docker-backed, in `rust/crates/baukit-auth/tests/postgres.rs`, each on a fresh PostgreSQL
container with the reference migration and a test owner table joined by a cascading foreign key:

- Atomic issue with grants: the grants round-trip through the store and reach
  `Principal::grants` through `ApiTokenVerifier`. A product CHECK that rejects a grant and a missing
  owner both fail the issue and leave no row.
- Active limit: ten concurrent issues against a limit of three give exactly three tokens and seven
  `PolicyRejected` results carrying the configured rejection. An expired token does not count, and
  another owner is unaffected.
- Revocation: a stranger's revoke gives `NotFound` and the token still verifies. After the owner
  revokes, verification gives `Invalid`, a second revoke gives `NotFound`, and the list still shows
  the revoked token.
- Expiry: a token verifies before expiry and records the use; at expiry it gives `Expired` and the
  recorded use does not move.
- Owner erasure: deleting the owner row cascades; `erase_owner_api_tokens` removes exactly one
  owner's two tokens and leaves another owner's.
- Concurrent last use: 32 concurrent `touch_last_used` calls with shuffled instants leave the
  latest one, and a later stale touch does not move it back.
- Purge: batches of one remove the token expired before the cutoff and the token revoked before it,
  and keep the active token, the token expiring after the cutoff, and the token revoked at the
  cutoff.

Unit tests in `api_token.rs` cover grant validation boundaries and the grants path through the
service. Verifier unit tests cover scope and profile claim parsing. `rust/crates/baukit-test/tests/
principal_claims.rs` covers step 3 against `MockOidcServer`: OIDC scopes and profile claims, a
custom array scope claim, Clerk and WorkOS without and with audiences, and one
`MultiIssuerVerifier` routing OIDC, Clerk, and WorkOS tokens, including the duplicate and empty
cases.

## Failure behavior

- Every SQL failure becomes `ApiTokenStoreError::Internal`, so the service returns
  `ApiTokenError::Storage` with the fixed display text. A product CHECK violation on grants is a
  storage error, not a validation error. Products that want a 400 validate grant names before
  calling `issue`.
- The active limit is the only policy rejection the store produces, and its content is the
  product's own `ApiTokenPolicyRejection`.
- A failed `touch_last_used` fails verification, unchanged from before.
- Malformed scope claims fail verification with `InvalidPrincipalContext`, which the Axum
  integration maps to the `invalid` bearer challenge.

## Privacy boundary

The store writes only the SHA-256 digest and a display prefix; the plaintext secret never reaches
SQL. Grants are product-defined strings and should not contain personal data. Profile claims cross
the auth boundary only when the product names them, and only as strings and booleans. The advisory
lock key is a hash of the owner UUID and appears only in `pg_locks`.

## Supported runtimes

Tokio with SQLx 0.9 against PostgreSQL. The container tests run the `baukit-test` PostgreSQL image.
The claim and grant changes have no runtime requirement beyond the existing verifier.

## Breaks

- `ApiToken`, `ApiTokenRecord`, and `NewApiToken` gain a public `grants` field. Struct literals
  break, including every product's store adapter.
- `ApiTokenError` gains `InvalidGrants`.
- `PrincipalClaimMapping::new()` and `default()` map `scope`. A token with a non-string,
  non-array `scope` claim now fails verification.

No existing field or wire name was renamed. No version changed.

## Product code to remove on adoption

- Eigenruhe: `crates/eigenruhe-postgres/src/api_tokens.rs` except `resolve_owner` and the keyset
  list page if the product keeps paging, `assign_scopes` and `scopes_for`, the issue-then-assign and
  revoke-on-failure path in `create_scoped`, and the `cardinality(scopes) > 0` lookup filter. The
  retention purge in the worker's `retention.rs` becomes `purge_inactive_api_tokens`.
- Hebkit: `crates/hebkit-postgres/src/adapters/postgres/api_tokens.rs` and `ApiTokenScopes::set`.
- Leitbild: `crates/leitbild-postgres/src/api_token.rs`, `TokenPermissionRepository`, and the
  issue-then-set-permissions path in `crates/leitbild-api/src/api_token.rs:185-230`. The
  delete-old-revoked step inside `create` becomes a scheduled `purge_inactive_api_tokens`.
- Tiefgang: the `IssuingStore` wrapper in `tiefgang-services/src/api_tokens.rs:69-106,173-250` and
  `create_scoped` in `tiefgang-postgres/src/api_tokens.rs:52-81`, once the `kind` question below is
  settled.
- Redemut: `unverified_issuer` at `redemut-auth/src/lib.rs:456`, `verified_token_has_audience` at
  `:492`, `verified_token_has_scope` at `:507`, and its profile claim decoding.
- Solo Leveling System: the second decode of email, `email_verified`, name, `preferred_username`,
  and picture in `sl-integrations/src/oidc.rs:64-87`.
- Schlauzug: the `scope` string parsing in `schlauzug-oidc/src/access.rs:205-250`.

## Product adoption follow-ups (deferred)

- Eigenruhe: copy the migration, rename `scopes` to `grants`, add the owner
  foreign key to `user_identities`, move the known-scope CHECK onto `grants`, and use
  `with_active_token_limit` with its existing rejection code.
- Hebkit: same as Eigenruhe, plus rename `user_id` to `owner_id` and keep the 80-character name
  limit as a product CHECK.
- Leitbild: rename `user_id` to `owner_id` and `permissions` to `grants`, and move the known-list
  CHECK.
- Tiefgang: decide whether `kind` stays as a product column on `api_tokens` with its per-kind CHECK,
  or becomes a grant. Then drop `IssuingStore`.
- Runtime Analyzer: stays on its own adapters because of tenant row-level security and hex digests.
  It can adopt `Principal::scopes` for OIDC callers.
- Redemut: switch to `MultiIssuerVerifier::from_verifiers` with `ClerkVerifier::with_audiences` and
  `WorkOsVerifier::with_audiences` where it needs `aud`, and read `Principal::scopes` and
  `Principal::profile_claim`.
- Solo Leveling System: read profile claims from `Principal::profile_claims`.
- Schlauzug: read `Principal::scopes` and `Principal::client_id` instead of reparsing claims.

## Follow-up studies

- Route credential policy layer. Several products decide per route whether an OIDC session, an API
  token, or both may call it, and which scope or grant is required. A study should compare those
  rules before Baukit adds a policy type, and decide whether scopes and grants should meet in one
  permission check or stay separate.
- `CredentialVault` store. Products that hold third-party credentials encrypt them at rest in
  their own tables. A study should compare key handling, rotation, and the row shapes before any
  shared store exists.

## Product defects found

- Eigenruhe, Hebkit, and Leitbild issue a token and then assign its scopes or permissions in a
  second write. A failure between the two leaves a token with no grants until the compensating
  revoke runs, and a crash skips the revoke entirely. The plan named two of these; Leitbild is the
  third.
- Tiefgang's active-token limit reads `list_for_owner` and then inserts, with no lock, so concurrent
  issues can exceed it.
- Every product overwrites `last_used_at` unconditionally, so a slow request can move it backwards.
- Hebkit and Leitbild count active tokens against the database `now()` instead of the service
  instant passed to `issue_at`, so a test clock and the limit disagree.
