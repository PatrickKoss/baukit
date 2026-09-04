# Evidence for item 16

- Source product files: `/home/patrick/projects/redemut/scripts/check-keycloak-policy.mjs`, `/home/patrick/projects/redemut/keycloak/realm.json`, and `/home/patrick/projects/redemut/.github/workflows/ci.yml`.
- Observed failure or repeated glue: Baukit's generated realm omitted password and brute-force rules. Redemut's checker inferred development status from realm text and did not check clients or redirect bounds.
- Baukit owner: the authenticated backend template and `baukit doctor`.
- Public types and errors: the JSON policy declaration accepts `development` or `production`; the script returns exit 1 and prints one line for each failed rule.
- Product-owned inputs: realm name, registration, exact password bounds and exclusions, TLS minimum, redirect count, application schemes, clients, and accounts.
- Concurrency, failure, privacy, and cleanup: validation is read-only and deterministic; malformed declarations fail closed; diagnostics omit realm credentials and client secrets; no cleanup is needed.
- Supported runtimes: Python 3 standard library and generated products using Keycloak 26.7.0.
- Product adoption change: Redemut can delete `scripts/check-keycloak-policy.mjs` and replace its CI invocation with the generated validator after adopting the release.
