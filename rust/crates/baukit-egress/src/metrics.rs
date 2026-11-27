use std::{borrow::Cow, time::Duration};

use axum::http::StatusCode;
use baukit_http::RetryClass;
use baukit_resilience::BreakerState;

use crate::EgressError;

/// Counter of outbound requests by `destination`, `status` and `outcome`.
pub const EGRESS_REQUESTS_TOTAL: &str = "egress_requests_total";
/// Histogram of the time to the response headers, by `destination`.
pub const EGRESS_REQUEST_DURATION_SECONDS: &str = "egress_request_duration_seconds";
/// Gauge of the worst breaker state per `destination`: 0 closed, 1 half-open, 2 open.
pub const EGRESS_CIRCUIT_STATE: &str = "egress_circuit_state";
/// Counter of breaker transitions by `destination` and `to`.
pub const EGRESS_CIRCUIT_TRANSITIONS_TOTAL: &str = "egress_circuit_transitions_total";

const NO_STATUS: &str = "none";

pub(crate) type Destination = Cow<'static, str>;

pub(crate) fn record_request(destination: &Destination, result: Result<StatusCode, &EgressError>) {
    let (status, outcome) = match result {
        Ok(status) => (status_label(Some(status)), "success"),
        Err(error) => (status_label(error_status(error)), outcome_label(error)),
    };
    metrics::counter!(
        EGRESS_REQUESTS_TOTAL,
        "destination" => destination.clone(),
        "status" => status,
        "outcome" => outcome
    )
    .increment(1);
}

pub(crate) fn record_duration(destination: &Destination, elapsed: Duration) {
    metrics::histogram!(EGRESS_REQUEST_DURATION_SECONDS, "destination" => destination.clone())
        .record(elapsed.as_secs_f64());
}

pub(crate) fn record_transition(destination: &Destination, to: BreakerState) {
    metrics::counter!(
        EGRESS_CIRCUIT_TRANSITIONS_TOTAL,
        "destination" => destination.clone(),
        "to" => to.as_str()
    )
    .increment(1);
}

pub(crate) fn record_state(destination: &Destination, worst: BreakerState) {
    metrics::gauge!(EGRESS_CIRCUIT_STATE, "destination" => destination.clone())
        .set(f64::from(worst.gauge_value()));
}

fn status_label(status: Option<StatusCode>) -> Cow<'static, str> {
    status.map_or(Cow::Borrowed(NO_STATUS), |status| {
        Cow::Owned(status.as_u16().to_string())
    })
}

const fn error_status(error: &EgressError) -> Option<StatusCode> {
    match error {
        EgressError::Status { status, .. } => Some(*status),
        _ => None,
    }
}

const fn outcome_label(error: &EgressError) -> &'static str {
    match error {
        EgressError::CircuitOpen { .. } => "circuit_open",
        EgressError::Saturated => "saturated",
        EgressError::Status {
            status: StatusCode::TOO_MANY_REQUESTS,
            ..
        } => "rate_limited",
        _ => match error.retry_class() {
            RetryClass::RateLimited => "rate_limited",
            RetryClass::RetryAfter(_) | RetryClass::Unavailable => "unavailable",
            RetryClass::Timeout => "timeout",
            RetryClass::Revoked => "revoked",
            RetryClass::Permanent => "permanent",
        },
    }
}
