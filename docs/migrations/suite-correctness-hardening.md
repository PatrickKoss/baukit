# Suite correctness hardening

Breaking changes in `baukit-suite` and `@baukit/suite-client`.

1. `StandaloneAccountPlan::new(product, entitlements, members)` takes a third
   argument. Pass `Arc::new(NoOrganizationMembers)` when the product has no
   organizations, or an adapter over the product's own member table. The old
   plan treated every subject as a member of every organization.
2. `validate_return_url` takes `allow_dev_scheme: bool`. The `<scheme>-dev`
   return is now off by default. Set `suite.allow_dev_scheme = true` (env
   `<PREFIX>__SUITE__ALLOW_DEV_SCHEME=true`) in local and dev configs only.
   baukit 0.30.6 and earlier accepted `<scheme>-dev://suite/linked` unconditionally. A
   0.30.x product whose dev build variant links through the suite must set the
   flag in that build's backend config, or `start` returns
   `suite_return_url_invalid` again. The template has no suite link config, so
   the product owns this setting.
3. Peers send `x-suite-signature-v2` next to the v1 signature. It covers the
   replay flag. A receiver now refuses `x-suite-replay` without v2 with 401, so
   an attacker cannot strip v2 and turn a replay into a live event. Deploy this
   version to every peer before running replays; an older sender's replays fail
   and retry until it upgrades. Release-blocked: refusing v1-only deliveries
   entirely needs every peer on this release first.
4. A suite event id reused with a different `event_type` or `occurred_at`
   returns `suite_event_id_conflict` instead of a duplicate.
5. `@baukit/suite-client`: `SuiteSession` checks the server's authorize URL.
   Pass `authorizeUrlPolicy: { allowedOrigins: [...] }` to pin it. Tickets with
   duplicate JSON keys or non-integer numbers now fail with `ticket_invalid`,
   as they already did in Rust.
6. `@baukit/purchases-core`: a refresh that returns no entitlement clears the
   held one. `useOrganizationPlan` reports `not_accessible` when the answer
   names a different organization.
