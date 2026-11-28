# BaukitPushDeliveryFailing

## Meaning

More than 10% of the push notifications the service sent in the last 15 minutes failed, for 15
minutes, with at least one notification every 100 seconds. `push_notifications_total` counts one
sample per notification per send attempt. Failed means outcome `rejected` (the provider refused it,
for example `MessageTooBig`, `InvalidCredentials` or an APNs/FCM error) or `failed` (the request to
the provider failed and the notification was not sent). `token_dead` is an uninstalled app and does not
count, and neither does `accepted`, which is still waiting for its receipt.

## Impact

Users miss notifications. A `failed` notification was not sent and the product's job retries it.
A `rejected` one is gone unless the rejection is retryable.

## First checks

Outcomes over time:

```promql
sum by (outcome) (rate(push_notifications_total{service="<service>", environment="<environment>"}[5m]))
```

Is the provider reachable?

```promql
sum by (outcome, status) (rate(egress_requests_total{service="<service>", environment="<environment>", destination="expo"}[5m]))
```

## Fix

- `failed` with egress `unavailable` or `timeout`: Expo is down or unreachable. Follow
  `BaukitEgressErrorRatioHigh` for `destination="expo"`.
- `failed` with egress `permanent` or a `4xx` status: the request itself is wrong. Check the release
  that started it and the access token configured for the sender.
- `rejected`: the sender gets a `PushRejection` per notification. Check what the product logs for
  it, or the Expo push tool for a sample ticket. `InvalidCredentials` means the APNs key or FCM
  credential in the Expo project is missing or expired. `MessageTooBig` is a product bug in the payload.

## Escalation

Product team. Contact Expo support when its status page shows no incident.
