# M-04 SuiteLink

Suite account linking for one link request between an initiator app I and an authorizer app A, with two
concurrent `complete` calls, exchange retries after a timeout, revocation and expiry. No known finding; the
model guards the protocol against regressions.

## What maps to what

| Model | Code (`rust/crates/baukit-suite/src/services/link.rs`) |
|---|---|
| `Start` | `start` |
| `Authorize` | `authorize` at A; the callback carries the code to I |
| `Expire` | request and code expiry (`SUITE_REQUEST_LIFETIME_SECONDS`, `SUITE_CODE_LIFETIME_SECONDS`) |
| `CompletePrepare` | `complete`, first transaction: `request_for_owner_for_update`, `validate_complete`, `prepared_exchange` |
| `ExchangeAtA` | `exchange_inner` at A in one transaction; `delivered = FALSE` is a timeout after A committed, and `exchange_with_peer` calls again |
| `Persist` | `persist_completed`: re-validates under the row lock, consumes, `replace`, `record_request_link` |
| `RevokeAtI`, `RevokeAtA`, `DeliverRevoke` | `disconnect`, `replace` and the peer revoke outbox |

## Invariants

- `AtMostOneLinkPerRequest`: one initiator link insert per request, one authorizer link insert per code.
- `ConsumedImpliesLinkId`: a consumed request records its link id.
- `CompletionIdempotent`: every `complete` that returns a link returns the same one.
- `InitiatorImpliesAuthorizer`: an active initiator link implies A created its side.

## Configs

| Config | Expect |
|---|---|
| `SuiteLink.cfg` | pass |

Two completers, up to 4 exchange calls. A probe invariant confirmed the model reaches the state where both
completes return the link, so the invariants are not vacuous.

## Paired test

The suite link service tests (concurrent completes, exchange replay after timeout) in
`rust/crates/baukit-suite/tests`.
