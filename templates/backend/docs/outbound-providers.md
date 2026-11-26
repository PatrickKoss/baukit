# Calling an outside provider

Every outbound HTTP call goes through `baukit_egress::GuardedClient`. Build one
client per provider and give it a destination name as its first argument:

```rust
let payments = GuardedClient::new("payments", EgressOptions::default())?;
```

The name labels the egress metrics and the circuit breaker, so a dashboard shows
which provider failed and a breaker opened by one provider never blocks another.
Use a short, stable, lowercase name for the provider, such as `payments` or
`mailer`. Never put a host, tenant or user id in it: each name becomes a metric
label value, and an unbounded set of names grows the label set without limit.
The platform receiver already uses `hub` for the suite hub.

## Open circuits in jobs

When a provider keeps failing, the client opens its circuit and returns
`EgressError::CircuitOpen { retry_after }` without sending the request. A full
bulkhead returns `EgressError::Saturated`. Neither means the job is wrong, so a
job handler should defer it instead of burning an attempt. Enable the `egress`
feature of `baukit-jobs` and map the error with `egress_job_error`:

```rust
use baukit_jobs::{JobError, egress_job_error};

let response = payments
    .execute(request)
    .await
    .map_err(|error| egress_job_error(&error))?;
```

`egress_job_error` turns an open circuit into `JobError::deferred` with the
breaker's delay and a saturated client into a short deferral. Deferrals do not
count against `max_attempts`; `WorkerConfig::max_deferrals` still caps them. A
non-retryable failure, such as a blocked address or a 4xx response, becomes
`JobError::permanent`. Other transport failures stay retryable and use the
provider's `Retry-After` when it sent one.
