# Boundary hardening

These changes tighten input checks at Baukit's trust boundaries. Most products need no code change. The table lists what to check.

| Crate | Change | What to do |
| --- | --- | --- |
| `baukit-egress` | `168.63.129.16` (Azure WireServer) is denied under every policy | Nothing, unless an operator endpoint pointed at it on purpose. That now fails with `EgressError::Blocked`. |
| `baukit-egress` | New `AllowedEndpoint::tenant` and `tenant_from_url` | Build endpoints from tenant-supplied URLs with these. They refuse loopback, private and metadata answers that `AllowedEndpoint::new` accepts for operator services. |
| `baukit-mcp` | Non-JSON POST bodies get `400 invalid_json`; bad bearers share a `429` bucket | Clients that retried on `401` in a tight loop now see `429` with `Retry-After`. Rate-limit buckets reset once because the key format changed. |
| `baukit-data-subject` | `ReceiptPayload` rejects unknown fields | Receipts signed by current Baukit verify unchanged. A verifier that must read receipts from a newer signer has to upgrade with it. |
| `baukit-credential-vault` | New `seal_record` / `open_record` (format v2) | Optional. New rows should use it. To migrate a row, `decrypt` it and `seal_record` it, then store `record_id` next to the fields. Old rows keep working with `decrypt`. |
| `baukit-http` | Strict CORS origins | Remove paths and trailing slashes from `cors_allowed_origins`. A non-loopback `http://` origin works only with `HttpOptions::from_config_in(config, DeploymentEnvironment::Local)`. |
| `baukit-http` | `X-Request-ID` filter | Clients sending ids outside `[A-Za-z0-9._-]{1,128}` get a server-generated UUID back. |
| `baukit-http` | `RevisionEtag` prefix may not end in a digit | Change a prefix like `"v1"` to `"v1-"`. `RevisionEtag::new` fails to compile with a digit-final constant. |
| `baukit-events`, `@baukit/events` | Same envelope shape in Rust and TypeScript; new `event_in_future` | Add a match arm for `EventInFuture`. Senders must emit `occurredAt` as `YYYY-MM-DDTHH:MM:SS[.fff]Z` with at most millisecond precision and must not pad `userId`. |
