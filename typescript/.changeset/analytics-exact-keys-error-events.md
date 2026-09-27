---
'@baukit/analytics-core': minor
---

Add exact-key scrubbing and `scrubErrorEvent` for crash reports. `scrubProperties` and `AnalyticsClient` accept `exactBlockedKeys`, which redact a key only when the normalized key is equal. `scrubErrorEvent` redacts request headers, bodies, query strings, breadcrumb data, and frame variables, and keeps event and trace IDs and stack frame locations. Break: `ip`, `ip_address`, `remote_addr`, `x_forwarded_for`, and `x_real_ip` are now redacted by default through `DEFAULT_EXACT_BLOCKED_KEYS`.
