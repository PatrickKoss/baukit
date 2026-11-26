//! Maps [`baukit_egress::EgressError`] to a [`JobError`].

use std::time::Duration;

use baukit_egress::EgressError;

use crate::JobError;

const SATURATED_DEFERRAL: Duration = Duration::from_secs(1);

/// Classifies a failed outbound call for a job handler.
///
/// An open circuit ([`EgressError::CircuitOpen`]) or a full bulkhead
/// ([`EgressError::Saturated`]) becomes [`JobError::deferred`], so waiting for
/// the destination does not consume an attempt. Every other error keeps its
/// retry class: a provider delay becomes [`JobError::retryable_after`], other
/// retryable failures [`JobError::retryable`], and refused destinations,
/// revoked credentials and permanent statuses [`JobError::permanent`].
///
/// The message holds only the stable [`EgressError::code`], never the URL.
///
/// ```
/// use std::time::Duration;
/// use baukit_egress::EgressError;
/// use baukit_jobs::egress_job_error;
///
/// let error = egress_job_error(&EgressError::CircuitOpen { retry_after: Duration::from_secs(30) });
/// assert_eq!(error.deferral(), Some(Duration::from_secs(30)));
/// ```
#[must_use]
pub fn egress_job_error(error: &EgressError) -> JobError {
    let message = format!("outbound call failed: {}", error.code());
    let class = error.retry_class();
    match error {
        EgressError::CircuitOpen { retry_after } => JobError::deferred(message, *retry_after),
        EgressError::Saturated => {
            JobError::deferred(message, class.retry_after().unwrap_or(SATURATED_DEFERRAL))
        }
        _ if !class.is_retryable() => JobError::permanent(message),
        _ => match class.retry_after() {
            Some(delay) => JobError::retryable_after(message, delay),
            None => JobError::retryable(message),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use baukit_egress::EgressError;

    use super::egress_job_error;

    #[test]
    fn open_circuit_defers_for_the_remaining_time() {
        let error = egress_job_error(&EgressError::CircuitOpen {
            retry_after: Duration::from_secs(12),
        });

        assert_eq!(error.deferral(), Some(Duration::from_secs(12)));
        assert!(error.is_retryable());
        assert_eq!(error.to_string(), "outbound call failed: circuit_open");
    }

    #[test]
    fn saturated_destination_defers_briefly() {
        let error = egress_job_error(&EgressError::Saturated);

        assert_eq!(error.deferral(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn transport_failures_consume_an_attempt() {
        for source in [EgressError::Timeout, EgressError::Resolve] {
            let error = egress_job_error(&source);

            assert_eq!(error.deferral(), None);
            assert!(error.is_retryable());
            assert_eq!(error.retry_after(), None);
        }
    }

    #[test]
    fn refused_destinations_are_permanent() {
        for source in [
            EgressError::BlockedAddress,
            EgressError::ResponseTooLarge { limit: 1 },
        ] {
            let error = egress_job_error(&source);

            assert!(!error.is_retryable());
            assert_eq!(error.deferral(), None);
        }
    }
}
