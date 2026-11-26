# Push install secrets

`baukit-push` now stores a SHA-256 digest of a per-install secret on each device row. A
device token moves to another owner only when that owner presents the same secret. This
stops one account from taking over another account's device by replaying its token.

Copy `rust/crates/baukit-push/migrations/0005_baukit_push_install_secrets.sql` into the
product's ordered migrations after `0004`. It adds a nullable column with no default, so
Postgres does not rewrite the table, and adds its length check `NOT VALID`.
`0007_baukit_push_validate_install_secret_hash.sql` validates that check in its own
transaction. Apply it after `0006`.

Then have the app generate a random secret once per install (16 to 256 visible ASCII
characters), keep it in secure storage, and send it with every registration. The backend
passes it on with `DeviceRegistration::with_install_secret`.

Rows registered before the change have no digest. They still move to a new owner as before,
and the registry logs a warning when that happens. The next registration with a secret
locks the row to it.

Handle `PushStoreError::TokenHeldByAnotherOwner` as a `409`. Code that builds
`DeviceRegistration` with a struct literal must switch to `DeviceRegistration::new`.

`PushSender::send` fails with `PushSendError`. Its `outcomes` lists notifications that were
already sent, so do not resend those; retry only its `unsent` messages.

## Delivery claim tokens

Apply `0006_baukit_push_delivery_claim_ids.sql` after `0005`. `DeliveryClaimStore::claim` now
returns `Option<ClaimToken>` instead of `bool`, and `release(claim, token)` needs that token.
Claims taken before the migration have no token, so they cannot be released and stay until
`purge` removes them.
