# Boundary hardening

These changes tighten input checks at Baukit's trust boundaries. Most products need no code change. The table lists what to check.

| Crate | Change | What to do |
| --- | --- | --- |
| `baukit-egress` | `168.63.129.16` (Azure WireServer) is denied under every policy | Nothing, unless an operator endpoint pointed at it on purpose. That now fails with `EgressError::Blocked`. |
| `baukit-egress` | New `AllowedEndpoint::tenant` and `tenant_from_url` | Build endpoints from tenant-supplied URLs with these. They refuse loopback, private and metadata answers that `AllowedEndpoint::new` accepts for operator services. |
| `baukit-mcp` | Non-JSON POST bodies get `400 invalid_json`; bad bearers share a `429` bucket | Clients that retried on `401` in a tight loop now see `429` with `Retry-After`. Rate-limit buckets reset once because the key format changed. |
| `baukit-data-subject` | `ReceiptPayload` rejects unknown fields | Receipts signed by current Baukit verify unchanged. A verifier that must read receipts from a newer signer has to upgrade with it. |
| `baukit-credential-vault` | New `seal_record` / `open_record` (format v2) | Optional for product code. New rows should use it. To migrate a row, `decrypt` it and `seal_record` it, then store `record_id` next to the fields. Old rows keep working with `decrypt`. |
| `baukit-auth`, `baukit-suite` | Stored secrets are sealed as v2 records with a new `record_id` column | Add the new migrations before deploying. See [Vault v2 for stored sessions and suite secrets](#vault-v2-for-stored-sessions-and-suite-secrets). |
| `baukit-http` | Strict CORS origins | Remove paths and trailing slashes from `cors_allowed_origins`. A non-loopback `http://` origin works only with `HttpOptions::from_config_in(config, DeploymentEnvironment::Local)`. |
| `baukit-http` | `X-Request-ID` filter | Clients sending ids outside `[A-Za-z0-9._-]{1,128}` get a server-generated UUID back. |
| `baukit-http` | `RevisionEtag` prefix may not end in a digit | Change a prefix like `"v1"` to `"v1-"`. `RevisionEtag::new` fails to compile with a digit-final constant. |
| `baukit-events`, `@baukit/events` | Same envelope shape in Rust and TypeScript; new `event_in_future` | Add a match arm for `EventInFuture`. Senders must emit `occurredAt` as `YYYY-MM-DDTHH:MM:SS[.fff]Z` with at most millisecond precision and must not pad `userId`. |

## Vault v2 for stored sessions and suite secrets

`baukit-auth` browser sessions and `baukit-suite` link secrets and PKCE verifiers now call
`seal_record` for every write. The record id goes into a new nullable column next to the
ciphertext. A row whose record id column is NULL was written before this release and still opens
through the v1 `decrypt` path. Nothing re-seals old rows.

Add these as new product migrations and apply them before deploying the new binary. The new
binary selects the columns, so it fails against a schema without them. The previous binary keeps
working against the new schema: it never names the columns, and the rows it writes open as v1.

| Crate | File | Constant | Adds |
| --- | --- | --- | --- |
| `baukit-auth` | `migrations/0003_baukit_auth_browser_session_record_ids.sql` | `browser_session::POSTGRES_BROWSER_SESSION_RECORD_IDS_MIGRATION_SQL` | `baukit_auth_browser_sessions.record_id` |
| `baukit-suite` | `migrations/005_suite_record_ids.sql` | `SUITE_RECORD_IDS_MIGRATION_SQL` | `suite_links.secret_record_id`, `suite_link_requests.verifier_record_id`, `suite_link_requests.secret_record_id` |

Both files only add nullable columns without defaults. They set `lock_timeout` and do not rewrite
the tables.

`EncryptedPayload` in `baukit-suite` gains `record_id: Option<Uuid>`. Code that builds the struct
by hand adds `record_id: None` for a v1 payload.

With v2, an edit to the database fails to open instead of decrypting:

- a token column set to NULL or copied from another session row,
- a record id copied from another row, or set to NULL to force the v1 path,
- a suite secret's ciphertext emptied or copied from another link.

### When the v1 read path goes away

Baukit keeps the v1 fallback until a release whose notes ship a re-seal job. It will not drop the
fallback earlier than the second minor release after this one. Before upgrading to that release,
each of these queries must return 0:

```sql
SELECT count(*) FROM baukit_auth_browser_sessions WHERE record_id IS NULL;
SELECT count(*) FROM suite_links WHERE secret_record_id IS NULL;
SELECT count(*) FROM suite_link_requests
WHERE verifier_record_id IS NULL OR (secret_ciphertext IS NOT NULL AND secret_record_id IS NULL);
```

Browser sessions reach 0 on their own once every v1 row is past `absolute_expires_at` and
`purge_browser_sessions` has run. Link requests live 10 minutes and the hourly cleanup removes them.
Suite links do not expire, so a v1 link secret stays until the link is replaced or a product
re-seals it.
