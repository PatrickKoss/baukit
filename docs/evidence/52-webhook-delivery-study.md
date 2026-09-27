# Webhook delivery study

Item 16 of the [cross-product feature plan](../cross-product-feature-plan.md), second deliverable.
The guarded client it builds on is in [52-guarded-egress.md](52-guarded-egress.md).

The plan asks for a comparison, a threat and failure matrix, and neutral ports and conformance
cases, with no delivery runtime yet. This note does that, settles the signature vectors, and
records the product defects the survey turned up.

## Source revisions

- Tiefgang `2d37a06`: webhook subscriptions and signing in
  `backend/crates/tiefgang-services/src/webhooks.rs`, destination rules in
  `tiefgang-domain/src/webhooks.rs:79-114`, delivery in `tiefgang-worker/src/lib.rs:80-171`.
- Runtime Analyzer `d47bfd5`: `finops-integrations/src/lib.rs:22-141` and
  `finops-integrations/src/webhook.rs:11-60` (v2 signing and `verify_signature_v2`).
- Solo Leveling System `3461eaf`: `sl-services/src/webhooks.rs:121-300`,
  `sl-notifications/src/http.rs`, `sl-notifications/src/channels.rs:75-90,210-265,310-325`,
  `sl-bin/src/bin/worker.rs:62-89`.
- Eigenruhe `f74cebb`: `eigenruhe-integrations/src/hub.rs:60-170` and its test vector at
  `hub.rs:252-273`.
- Hebkit `841bf5d` and Tiefgang's receiver, for comparison only: Hebkit sends suite events with a
  bearer token (`hebkit-services/src/suite_events.rs:315-370`), and Tiefgang dedupes them on
  owner, source app, and event ID (`tiefgang-api/src/events_handlers.rs`).
- Baukit: `rust/crates/baukit-test/src/webhook.rs` and
  [integration reliability](../platform/integration-reliability.md) section 5.

## Comparison

| Dimension | Tiefgang | Runtime Analyzer | Solo Leveling System | Eigenruhe |
|---|---|---|---|---|
| Subscription owner | user | tenant | organization, no production create API or emitter | one operator-configured hub URL |
| Secret | 32 random bytes, hex, in the credential vault | user-supplied, at least 32 bytes | per subscription | one secret per user |
| Rotation | replaced at once, no overlap | replaced at once | none | none |
| Event identity | `Idempotency-Key` = event ID, stable across retries | `X-FinOps-Event-Id` = SHA-256 of the body, changes when the body is rebuilt | none | event ID inside the signed body |
| Signature input | Baukit v1: version, timestamp, delivery ID length, delivery ID, body | `v2:` + timestamp + `:` + body | body only | body only |
| Signature header | `X-Tiefgang-Signature: v1=` base64url | `X-FinOps-Signature-V2: v2=` hex | `X-Signature-256: sha256=` hex | `X-Eigenruhe-Signature: sha256=` hex |
| Timestamp | signed | signed | none | sent, not signed |
| Destination guard | yes | yes | none | none; `http` accepted |
| Redirects | off | off | followed | followed |
| Retry classes | `classify_http_status` | own classifier | every non-`2xx` | `classify_http_status` |
| `Retry-After` | uncapped | clamped to 1 to 300 s | ignored | uncapped |
| Attempts | 5 per job | per job, one job fans out to every channel | backoff 60, 300, 1800, 7200 s | 3 in process, 5 per job |
| Disable rule | after 20 attempts | none | after 5 attempts | none |
| Response body | never read | classified, discarded | stored, up to 2000 characters | parsed as JSON on `2xx` |

Only Tiefgang already uses the Baukit signature. The four senders use four signature inputs, and
two sign the body alone.

## Threat and failure matrix

"Covered" names the Baukit piece that handles the row today. "Open" rows need a product fix or a
later Baukit contract.

| Threat or failure | Tiefgang | Runtime Analyzer | Solo Leveling System | Eigenruhe | Baukit |
|---|---|---|---|---|---|
| Destination is private, loopback, or metadata | blocked | blocked | reachable | reachable | covered: `GuardedClient`, address vectors |
| DNS answer changes after the check | pinned | pinned | reachable | reachable | covered: pinned per connection, changed-answer test |
| Redirect to an internal host | refused | refused | followed | followed | covered: redirects returned as `Permanent` |
| Proxy from the environment | ignored | ignored | used | used | covered: `no_proxy` |
| Slow or huge receiver response | 10 s, body unread | 10 s | unbounded body | unbounded body | covered: timeouts and body limit |
| Secret or token in a URL leaks to logs or storage | no | no | leaks: reqwest error strings with the full URL are stored | no | covered for the client: errors and telemetry carry no URL |
| Receiver body stored | no | no | yes, 2000 characters | parsed | recipe: discard after classification |
| Forged request | HMAC | HMAC | HMAC | HMAC | covered: v1 signature and vectors |
| Replay of a captured request | timestamp signed | timestamp signed, but no route verifies it | nothing to check | timestamp unsigned, can be replaced | open: receiver window is receiver policy |
| Field-boundary ambiguity in the signed input | length-prefixed | `:` separated, timestamp is digits so unambiguous | body only | body only | covered: length prefix, vector pair `newline-in-*` |
| Duplicate delivery after a timeout | same event ID | new event ID if the body changes | no ID | event ID in body | open: stable delivery ID conformance case |
| Retry storm from a large `Retry-After` | uncapped | clamped | ignored | uncapped | open: cap decision |
| `425 Too Early` | permanent | own classifier | retried | permanent | open: recipe and classifier disagree |
| Disable counts attempts, not failed jobs | counts attempts | none | counts attempts | none | recipe: count failed jobs |
| Event dropped by a throttle | no | 5-minute throttle drops events | no | no | open: product fix |
| `2xx` with an unreadable body | delivered | delivered | delivered | dead-lettered as `hub_response_invalid` | open decision, see below |
| Secret rotation gap | no overlap | no overlap | no rotation | no rotation | recipe: current and previous key by key ID |
| DNS failure | permanent | retryable | retried | retried | covered: `Resolve` is `Unavailable` |

## Ambiguous delivery outcomes

Three outcomes do not say whether the receiver acted:

1. A timeout or connection reset after the request was written. `GuardedClient` reports
   `Timeout` or `Transport`, both retryable. The sender must retry with the same delivery ID,
   timestamp, body, and signature, and the receiver must dedupe on the delivery ID.
2. A `2xx` whose body the sender cannot parse. A webhook is fire and forget: `2xx` means
   delivered, and the sender never reads the body. Eigenruhe's hub is a request and response API
   dressed as a webhook, so its dead-letter is a product choice, not a delivery rule. A shared
   delivery contract should treat any `2xx` as delivered.
3. A `3xx`. The guarded client never follows it, and the recipe records it as permanent. The
   receiver moved; retrying the old URL cannot succeed.

## Signature vectors

The study supports settling on Baukit v1. It is the only input among the four that binds the
timestamp and a stable delivery ID to the body, and its length prefix removes field-boundary
ambiguity. Body-only signatures (Solo Leveling System, Eigenruhe) cannot stop replay, and Runtime
Analyzer's event ID is not signed and not stable.

`fixtures/webhooks/signature-v1.json` now publishes 8 signing cases with the exact signing bytes
and signatures, and 13 verification cases. They cover the existing reference vector, an empty
body, an empty delivery ID, a multibyte delivery ID (length counts bytes), a newline moved between
delivery ID and body (different signatures), a negative timestamp, a binary body, rotation with the
previous key, and rejections for a changed timestamp, delivery ID, or body, a missing or uppercase
prefix, padded base64url, hex, truncation, and an empty value. A Python `hmac` script generated the
file and `rust/crates/baukit-test/tests/webhook_signature_vectors.rs` checks that
`webhook_signing_input`, `sign_webhook_hmac_sha256`, and `verify_webhook_hmac_sha256` agree with
it. `webhook.rs` itself did not change.

I did not add a timestamp window or delivery-ID dedupe to `baukit-test`. Both are receiver policy
with product-owned storage, and the inbox conformance check already covers dedupe. A window helper
would be one comparison that every receiver writes anyway.

## Proposed neutral ports

These are proposals for a later item. None is implemented. They carry no event schema, no
subscription table, and no product vocabulary.

- `WebhookSigner`: `sign(key_id, timestamp, delivery_id, body) -> SignedHeaders`, with the v1
  signature as the only built-in version. Header names stay product configuration.
- `WebhookKeySource`: `current(owner) -> (KeyId, Secret)` and `candidates(owner, key_id)` for
  verification, backed by `baukit-credential-vault` in products that use it.
- `DeliveryOutcome`: a pure mapping from `Result<EgressResponse, EgressError>` to `Delivered`,
  `RetryAt(delay)`, or `Failed(code)`, with the `Retry-After` cap and the `425` choice as inputs.
- `DeliveryLedger`: records one outcome per job and the consecutive failed-job count per
  subscription, so the disable rule counts jobs rather than attempts.

A delivery runtime crate stays gated on the plan's condition: two products adopt one transport
contract without importing each other's event schemas or subscription tables.

## Proposed conformance cases

For a sender, run against `ScriptedWebhookReceiver` and a controllable resolver:

1. Every retry of one delivery sends the same delivery ID, timestamp, body, and signature.
2. A timeout after the request is written is retried with the same delivery ID.
3. `2xx` with any body is delivered; the body is not read.
4. `3xx` is permanent and the `Location` is never requested.
5. `429` with `Retry-After` schedules no earlier than the header and no later than the cap.
6. A private, mixed, or changed DNS answer fails with `blocked_address` before a connection.
7. Rotation: after a key change, the receiver verifies with the new key and, within the overlap,
   the previous one.
8. The disable counter moves once per failed job and resets on success.
9. No stored outcome, log line, or span holds the destination path, query, or user info, or the
   receiver body.

For a receiver: the published signature vectors, a timestamp outside the window rejected before
persistence, and a repeated delivery ID answered from the inbox without a second domain write.

## Product defects

- Solo Leveling System stores reqwest error strings, which contain the full destination URL, as
  the failure message (`channels.rs:75-90` with `http.rs:39,44`). Slack and Discord webhook URLs
  are bearer credentials, and the Telegram URL embeds the bot token
  (`channels.rs:320-321`). Receiver bodies up to 2000 characters are stored too.
- Solo Leveling System sends webhooks and user-supplied Slack and Discord URLs through
  `reqwest::Client::default()` (`http.rs:23-26`): no destination check, redirects followed, proxies
  honored, response body unbounded.
- Runtime Analyzer's `X-FinOps-Event-Id` is a hash of the body, so a receiver cannot dedupe a retry
  whose body was rebuilt; the 5-minute throttle drops events instead of delaying them; its DNS
  lookup has no timeout (`lib.rs:40-43`); `verify_signature_v2` has a replay window, but no route
  calls it.
- Eigenruhe sends `X-Eigenruhe-Timestamp` outside the signature, accepts `http` hub URLs, follows
  redirects, reads the whole `2xx` body, and dead-letters a `2xx` it cannot parse
  (`hub.rs:60-160`). Its companion ticket in Solo Leveling System expects a base64url-decoded key
  and a key ID; the code uses the raw secret and sends no key ID.
- Tiefgang disables a subscription after 20 attempts rather than 20 failed jobs, treats a DNS
  failure as permanent, and replaces a rotated secret with no overlap.
- Tiefgang and Eigenruhe pass `Retry-After` through uncapped.

## Decisions

- The study goes in this separate note because it is as long as the egress evidence.
- Baukit v1 is the recommended signature for new senders. Products keep their current headers
  until they adopt, and each adoption is a receiver-visible break for that product.
- No delivery runtime, no new public type in `baukit-test`, and no change to
  `classify_http_status` in this item.

## Breaks

None. The signature vectors are a new fixture and a new test.
