# Suite correctness hardening

Breaking changes in `baukit-suite` and `@baukit/suite-client`.

1. `StandaloneAccountPlan::new(product, entitlements, members)` takes a third
   argument. Pass `Arc::new(NoOrganizationMembers)` when the product has no
   organizations, or an adapter over the product's own member table. The old
   plan treated every subject as a member of every organization.
2. `validate_return_url` takes `allow_dev_scheme: bool`. The `<scheme>-dev`
   return is now off by default. Set `suite.allow_dev_scheme = true` (env
   `<PREFIX>__SUITE__ALLOW_DEV_SCHEME=true`) in local and dev configs only.
3. Peers send `x-suite-signature-v2` next to the v1 signature. It covers the
   replay flag. A receiver ignores `x-suite-replay` without v2. Deploy receivers
   before senders; a v1-only sender's replays now apply live rewards as normal
   events, so stop replays until every peer runs this version.
4. A suite event id reused with a different `event_type` or `occurred_at`
   returns `suite_event_id_conflict` instead of a duplicate.
5. `@baukit/suite-client`: `SuiteSession` checks the server's authorize URL.
   Pass `authorizeUrlPolicy: { allowedOrigins: [...] }` to pin it. Tickets with
   duplicate JSON keys or non-integer numbers now fail with `ticket_invalid`,
   as they already did in Rust.
6. `@baukit/purchases-core`: a refresh that returns no entitlement clears the
   held one. `useOrganizationPlan` reports `not_accessible` when the answer
   names a different organization.
