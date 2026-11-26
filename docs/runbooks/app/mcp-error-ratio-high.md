# BaukitMcpErrorRatioHigh

## Meaning

More than 5% of the requests to the service's `/mcp` endpoint returned 5xx for 10 minutes, with at
least one MCP request every 20 seconds. `mcp_requests_total` counts every request to `/mcp` by exact
status code.

## Impact

MCP clients (agents and tools) fail their tool calls. The rest of the API can be healthy.

## First checks

Codes over time:

```promql
sum by (status) (rate(mcp_requests_total{service="<service>", environment="<environment>"}[5m]))
```

Is token introspection failing? `unavailable` means the authorization server did not answer:

```promql
sum by (outcome) (rate(mcp_authentication_policy_decisions_total{service="<service>", environment="<environment>"}[5m]))
```

Errors in the logs:

```logql
{service="<service>", environment="<environment>", level="error"} | json | message=~"(?i).*mcp.*"
```

## Fix

- 503 with `unavailable` decisions: the IdP or its introspection endpoint is down. Fix that side.
- 500 from one tool: fix the tool handler; a tool given bad arguments should return a tool error, not
  a 5xx.
- 5xx on every route, not only `/mcp`: follow `BaukitHighErrorBudgetBurnFast`.

## Escalation

Product team. Page if MCP is a contracted integration for the product.
