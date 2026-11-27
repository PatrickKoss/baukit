# BaukitEgressLatencyHigh

## Meaning

The p95 time from sending an outbound request to receiving the response headers has been above 2 s for
10 minutes for one destination. `egress_request_duration_seconds` covers only requests that were sent;
calls refused by the breaker or the in-flight cap are not in it.

## Impact

Inbound requests that call the destination get slower and may hit their own deadline. The outbound call
is capped by the time left before the inbound request deadline, so these show up as `timeout` outcomes.
Slow calls also hold in-flight slots, so `saturated` may follow.

## First checks

Latency per destination:

```promql
baukit:egress_request_duration_seconds:p95_5m{service="<service>", environment="<environment>"}
```

Are timeouts rising with it?

```promql
sum by (outcome) (rate(egress_requests_total{service="<service>", environment="<environment>", destination="<destination>"}[5m]))
```

Slow traces: search spans named `egress.request` with a long duration for the service.

## Fix

- One destination slow, others fine: the provider is degraded. Check its status page. Five consecutive
  timeouts open the breaker, which sheds the load.
- Every destination slow: the problem is on our side (CPU starvation, DNS, egress proxy or NAT). Check
  `process_cpu_seconds_total` and the node.
- Slow only for large responses: the time includes connection setup and headers, not body transfer.
  Look at the destination's server time instead.

## Escalation

Product team. Contact the provider when its status page shows no incident.
