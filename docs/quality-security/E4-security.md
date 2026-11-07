# E4 security posture review: baukit and 12 products

Scope: static review only (rg, reading code). No builds, no exploits, no secret values printed.
Depth on baukit crates, TS packages, templates, deploy; wide greps over products.

## Summary

The Rust core is in better shape than I expected. baukit-auth refuses symmetric algorithms, requires
`kid`, `exp`, issuer and audience checks, and uses `subtle` for compares. The credential vault uses
AES-256-GCM with random 96-bit nonces and AAD binding scope, key version and field name. The njs media
grant verifier is strict and constant-time. The Helm chart already sets runAsNonRoot, read-only root FS,
drop ALL, seccomp and NetworkPolicies. The real gaps are on the browser side (tokens in localStorage, no
CSP or security headers anywhere), in supply chain hygiene (no SHA pinning in 13 repos, no secret
scanning, no JS audit), and in testing practice (zero fuzz or property tests in baukit itself, no
cross-tenant negative tests in several products).

## A. Confirmed weaknesses (ranked)

### E4-01 auth-web keeps access and refresh tokens in localStorage, with no CSP to contain XSS
Severity: high. Exploit: any XSS in a generated web app (or a compromised npm dependency) reads
`localStorage[<prefix>:tokens]` and exfiltrates the refresh token, giving long-lived account takeover.
Evidence:
- `typescript/packages/auth-web/src/index.ts:172` `this.tokenStorageKey = \`${this.config.storageKeyPrefix}:tokens\`;`
- `typescript/packages/auth-web/src/index.ts:428` `this.environment.localStorage.setItem(this.tokenStorageKey, JSON.stringify(tokens));`
- `StoredTokens` built at `index.ts:415-422` includes `refreshToken` and `idToken`.
- No `Content-Security-Policy` in `templates/`, `deploy/`, `rust/crates`, or `typescript/packages` (rg -il returns nothing).
Affected: baukit auth-web and every product with a web client generated from `templates/web/__auth__`.
PKCE S256 is correct (`oidc-helpers.ts:33-34`), so the issue is storage, not the flow.
Fix: keep tokens in memory only, and hold the refresh token in a BFF httpOnly SameSite=Strict cookie
or rely on the provider's silent renew (the silent-token-broker already exists). If localStorage must
stay, drop the refresh token from storage and ship a strict CSP (below, E4-02).
Prevention: unit test in auth-web asserting no stored value contains `refresh_token`; semgrep rule
banning `localStorage.setItem` with token-named keys in `typescript/packages/auth-*`.

### E4-02 No security response headers in baukit-http or the web templates
Severity: med. Exploit: a generated web app can be framed (clickjacking on account deletion or org
admin flows) and has no CSP to blunt E4-01; API responses lack `nosniff`.
Evidence: `rust/crates/baukit-http/src/middleware.rs:159` adds `RequestBodyLimitLayer`, `:207-210`
configures CORS, but rg for `x-content-type-options|strict-transport|SetResponseHeader` in
`rust/crates/baukit-http/src` finds nothing. No CSP or `frame-ancestors` in `templates/web`.
Affected: baukit-http, all products.
Fix: add a default `SecurityHeadersLayer` in baukit-http (`X-Content-Type-Options: nosniff`,
`Referrer-Policy: no-referrer`, `Cache-Control: no-store` on authenticated JSON, HSTS behind a flag)
and a CSP plus `frame-ancestors 'none'` at the web edge (Traefik middleware in the chart or the
template's static server).
Prevention: integration test in baukit-http asserting headers on every route; generated-fixture
check that `curl -I` on the web build has CSP.

### E4-03 GitHub Actions pinned by mutable tag in every repo
Severity: med. Exploit: a compromised action tag (tj-actions style) runs in `release.yml` jobs that
hold `contents: write` and `id-token: write` (crates.io/npm trusted publishing), and can push or
publish a backdoored baukit that all 12 products then consume.
Evidence: `.github/workflows/release.yml:30-32`, `:96-97`, `:153-155` grant write and id-token;
actions such as `actions/checkout@v7.0.1`, `peter-evans/create-pull-request@v8.1.1` (release.yml:79),
`dtolnay/rust-toolchain@stable` (ci.yml:94), `taiki-e/install-action@cargo-deny` (ci.yml:150) are
tag or branch refs. Counts of 40-hex SHA pins: baukit 0/80, hebkit 1/52, lesezeichen 1/20, all other
products 0. No repo uses `pull_request_target` (good). Top-level `permissions:` is present everywhere
except `architecture-health-platform/.github/workflows/architecture-health-fixture.yml`.
Fix: pin all actions to SHAs with the tag in a comment; renovate (`renovate.json` exists in baukit)
can keep them current via `helpers:pinGitHubActionDigests`. Add `permissions:` to the AHP workflow.
Prevention: zizmor in CI (catches unpinned uses, excessive permissions, template injection).

### E4-04 Supply chain checks cover only `rust/`: no cargo-deny for `cli/`, no JS audit
Severity: med. Exploit: a vulnerable crate in the CLI (which renders templates and writes files on
developer machines) or a malicious/vulnerable npm package in `typescript/` ships without any gate.
Evidence: `rust/deny.toml` exists, `cli/deny.toml` does not; `scripts/ci-gate.sh:23` runs one
`cargo-deny` step; no `pnpm audit`, osv-scanner, gitleaks, trufflehog, semgrep or CodeQL referenced in
`.github`, `scripts`, `Makefile`. `rust/deny.toml` has `unknown-git = "deny"` and `wildcards = "deny"`
(good) but `multiple-versions = "warn"` and no `[bans] deny` list.
Products: only kassensturz, runtime-analyzer and architecture-health-platform have a `backend/deny.toml`;
eigenruhe, hebkit, leitbild, redemut, schlauzug, tiefgang, solo-leveling-system, lesezeichen,
schluesselbund have none.
Fix: run cargo-deny on `cli/` and on every product backend (template should emit `deny.toml` and a CI
step); add `osv-scanner` over all lockfiles (Cargo.lock + pnpm-lock.yaml) in one step.
Prevention: generated-fixture gate asserts `deny.toml` exists and passes.

## B. Hardening suggestions (no exploit confirmed)

### E4-05 Tenant scoping is per-query and per-service discipline, not enforced by type or RLS
Severity: med (latent). Scenario: a new handler calls a by-id repository method and forgets the org
filter, giving cross-tenant reads (IDOR).
Evidence: `solo-leveling-system/backend/crates/solo-leveling-system-postgres/src/teams.rs:114`
`SELECT {TEAM_COLUMNS} FROM teams WHERE id = $1` has no org filter; the check lives one layer up in
`solo-leveling-system-services/src/orgs.rs:366-371` (`.filter(|team| team.organization_id == organization_id)`).
Correct today, but nothing stops a second caller. Similar id-only lookups:
`eigenruhe/backend/crates/eigenruhe-postgres/src/coaching.rs:169`, `.../sync.rs:924`,
`tiefgang/backend/crates/tiefgang-postgres/src/guard.rs:739`,
`solo-leveling-system-postgres/src/wellness.rs:239`, `onboarding.rs:149`.
Rough count of files with cross-tenant style tests (rg `other_user|cross_tenant|another_org|foreign_org|other_org`):
eigenruhe 0, tiefgang 0, lesezeichen 0, kassensturz 0, redemut 1, schluesselbund 1, AHP 33.
Fix: make repository methods take `(OrgId, Id)` newtypes so an unscoped lookup does not compile;
consider Postgres RLS with `SET LOCAL app.org_id` for the highest-value tables.
Prevention: a baukit-test helper `assert_cross_tenant_denied(router, route, user_a, user_b)` run over
every route in the OpenAPI document (the products already have openapi_drift); a semgrep rule flagging
`WHERE id = $1` without `owner_id|user_id|organization_id` on tables that have such a column.

### E4-06 `IssuedApiToken` derives Debug with the plaintext secret
Severity: low. Scenario: `tracing::debug!(?issued)` or an error chain prints a live API token.
Evidence: `rust/crates/baukit-auth/src/api_token.rs:79-85` `#[derive(Clone, Debug, Eq, PartialEq)] pub struct IssuedApiToken { ... pub secret: String }`.
Contrast `CredentialCipher`, which has a hand-written redacting Debug (`credential-vault/src/cipher.rs:186-193`).
Fix: wrap in `secrecy::SecretString` or write a redacting Debug.
Prevention: clippy/semgrep rule: `derive(Debug)` on structs with fields matching `secret|token|password|api_key`.

### E4-07 OIDC audience check skipped when audiences are empty
Severity: low. `rust/crates/baukit-auth/src/verifier.rs:1028-1030` only calls `validate_audience` if
`config.audiences` is non-empty. `OidcConfig::new` requires one (`config.rs:235`), but the Clerk,
client-bound and scope-bound constructors set `audiences: BTreeSet::new()` (`config.rs:263`, `285`, `309`)
and rely on azp/client_id/scope instead. That is defensible for those providers, but a token minted
for another app on the same Clerk instance with no `azp` passes (`verifier.rs:1034` accepts `None`).
Fix: document the trade-off, and make Clerk's missing-azp acceptance opt-in.
Prevention: negative unit tests per profile: token for another client, token with no azp.

### E4-08 Products build raw reqwest clients outside baukit-egress
Severity: low (URLs are config, not user input, in the cases checked). Evidence:
`eigenruhe/backend/crates/eigenruhe-bin/src/manifest_source.rs:48`,
`eigenruhe/backend/crates/eigenruhe-integrations/src/fitness_tracker.rs:152`,
`tiefgang/backend/crates/tiefgang-bin/src/adapters/integrations/github.rs:29`, `toggl.rs:31`,
`redemut/backend/crates/redemut-llm-coach/src/lib.rs:37`, `redemut-elevenlabs/src/lib.rs:31`,
`redemut-auth/src/lib.rs:186`, `kassensturz/backend/crates/kassensturz-api/src/adapters/push_sender.rs:43`,
`kassensturz-fints/src/lib.rs:161`, `solo-leveling-system-ai/src/client.rs:71`.
These skip egress allowlists, private-IP blocking and redirect policy. If any base URL ever becomes
tenant-configurable (fitness_tracker takes a base URL), it becomes SSRF.
Fix: route through baukit-egress `GuardedClient`; clippy `disallowed-methods` for `reqwest::Client::new`
and `reqwest::Client::builder` outside an allowlisted adapter module.

### E4-09 Committed key material and env files (test-only, but worth a scanner allowlist)
Severity: low. `git ls-files` shows test PEMs (`baukit/fixtures/data-subject/v1/test-only-private.pem`,
`baukit/rust/crates/baukit-test/src/jwt/fixtures/oidc-key-1.pem`, runtime-analyzer `fixtures/evidence/tsa/ec/*.key.pem`,
AHP `tests/fixtures/github-app-test-only.pem`) and `leitbild/web/.env.production` / `.env.testing`
(key names only checked: `VITE_ANALYTICS_PROVIDER`, `VITE_CLERK_PUBLISHABLE_KEY`, `VITE_POSTHOG_API_HOST`,
all public by design). Nothing looks like a production secret, but without gitleaks nothing would catch one.
Fix: gitleaks with a `.gitleaks.toml` allowlisting these fixture paths.

## C. Things checked and found sound (so nobody re-reviews them)
- JWT: RS/ES/PS allowlist only, symmetric excluded (`config.rs:37-40`), `kid` required (`verifier.rs:611`),
  missing-kid refresh (`verifier.rs:643`), `exp` required (`verifier.rs:968`), nbf checked, skew 60s/5s/0.
- ID token nonce compare is constant-time (`verifier.rs:617`); API tokens hashed and compared with `ct_eq` (`api_token.rs:810`, `secret.rs:10-11`).
- redemut parses unverified claims only to route to a verifier (`redemut-auth/src/lib.rs:199-213`, `310-316`), then verifies.
- Vault: AES-256-GCM via ring, random nonce per field, AAD = scope+version+name, zeroize on error (`cipher.rs:158-182`, `214-240`).
- Media grants: fixed query shape, canonical base64url, expiry cap, constant-time HMAC compare (`deploy/media-grants/njs/media-grant.js:125-189`).
- MCP: bearer via `IdentityVerifier` plus exact-origin allowlist (`baukit-mcp/src/security.rs:30-47`, `111-113`, `config.rs:89-93`).
- Helm: securityContext and NetworkPolicy present (`deploy/chart/baukit-app/templates/redis-deployment.yaml:36-58`, `networkpolicy.yaml`); Dockerfile template runs as `USER 65532:65532` (`templates/backend/backend/Dockerfile.jinja:50`). Base images come from build args, so digest pinning is up to the caller; worth pinning defaults.
- No webhook/HMAC compare with `==` found in baukit or products (rg sweep).
- `format!` SQL in products interpolates only constant column lists/table names, all values bound.

## D. Missing practices

### Security testing today
- baukit: 0 fuzz targets, 0 files using proptest (rg over `rust/` and `cli/`).
- Products: kassensturz has cargo-fuzz (`backend/fuzz`, `scripts/fuzz-statements.sh`, `.github/workflows/statement-fuzz.yml`) and proptest in 20 files: this is the prior art to copy. runtime-analyzer has a fuzz crate. AHP 9 proptest files. eigenruhe, hebkit, schlauzug, tiefgang have no proptest.

### Ranked new fuzz/property targets in baukit
1. `baukit-auth` JWT path: `verifier.rs` header/claims decode and `validate_token_profile` (`:1020`). Untrusted bearer on every request. Property: never Ok without a valid signature; fuzz the decode path for panics.
2. `baukit-sync` push/pull request decoding and merge logic. Client-controlled batches; property tests for idempotency and owner isolation (no write lands under another user).
3. `baukit-events` envelope parsing and the TS `events/src/index.ts:80` user check. Cross-language differential test with `fixtures/` vectors.
4. `baukit-mcp` JSON-RPC dispatch (`server.rs`) and `security.rs` origin/bearer checks: fuzz request bodies; property: unauthenticated request never reaches a tool.
5. `deploy/media-grants/njs/media-grant.js` `parseGrant`/`verifyMediaGrant`: property test that Rust signer and njs verifier agree, and that any single-byte mutation is rejected.
6. `baukit-credential-vault` `cipher.rs` decrypt: roundtrip property, and that swapping scope/name/version AAD always fails.
7. `baukit-http` idempotency key and locale parsing (`idempotency.rs`, `locale.rs`): header parsers, cheap to fuzz.
8. `baukit-suite` link service validation (`services/link.rs:212`, `636`).
9. `baukit-config` env parsing: lower risk, operator input.

### Auth negative tests
Add a reusable `baukit-test` suite: expired, wrong iss, wrong aud, alg=none, HS256 with public key as secret, unknown kid, tampered payload, cross-tenant read/write per route. Generate it into every product via the template.

### Threat modeling
No threat model doc in baukit. Found mentions only in hebkit (1 md), kassensturz (3), AHP (2). A reusable
agent skill (in `agent-skills/`) should ask for: data classes and where they live, trust boundaries
(browser, mobile, backend, MCP, njs edge, third-party webhooks, egress targets), identity providers and token
types, tenant model, STRIDE per boundary, and output a `docs/threat-model.md` with a table mapping each threat
to a baukit control and a test. Re-run on any new route group or integration.

### Tooling fit
- zizmor + actionlint: high value, cheap; directly catches E4-03.
- gitleaks: high value; no secret scanning anywhere today.
- osv-scanner: one tool for Cargo.lock and pnpm-lock.yaml; fills the JS gap. Prefer it over `pnpm audit`.
- cargo-deny vs cargo-audit: keep cargo-deny (covers advisories plus licenses and sources); extend to `cli/` and products. cargo-audit adds nothing.
- semgrep with custom rules: worthwhile for the project-specific invariants (raw reqwest, Debug on secrets, unscoped `WHERE id = $1`, localStorage tokens). Generic rulesets add noise.
- clippy `disallowed-methods`/`disallowed-types` via `clippy.toml`: cheapest way to enforce egress and secret types in Rust.
- trivy/grype: useful on built images in the release pipeline only.
- eslint-plugin-security: low signal for this codebase; skip.
- CodeQL: moderate; Rust support is newer. Run it if hosted CI credits allow; not a priority.
