//! Connection health, as an owner and an operator need to see it.

use baukit_http::RetryClass;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a connection stands between working and needing the owner.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionHealth {
    /// The last import succeeded.
    #[default]
    Healthy,
    /// Imports are failing but retrying can still recover the connection.
    Degraded,
    /// The credential was rejected; only the owner reconnecting fixes this.
    NeedsReconnect,
    /// Retrying will not help and the failure is not an authorization problem.
    Failed,
}

impl ConnectionHealth {
    /// Returns the health a failure of this class leaves the connection in.
    pub const fn after_failure(class: RetryClass) -> Self {
        match class {
            RetryClass::Revoked => Self::NeedsReconnect,
            RetryClass::Permanent => Self::Failed,
            _ => Self::Degraded,
        }
    }

    /// Returns whether the owner has to act before imports resume.
    pub const fn needs_owner_action(self) -> bool {
        matches!(self, Self::NeedsReconnect)
    }

    /// Returns the stable label for metrics and operations output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::NeedsReconnect => "needs_reconnect",
            Self::Failed => "failed",
        }
    }
}

/// What a product shows for one connection on a status screen.
///
/// Every field is safe to persist and to render. `last_error_code` is the
/// [`ConnectorError::code`](crate::ConnectorError::code) of the most recent
/// failure, never provider body text.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionStatus {
    /// Current health.
    pub health: ConnectionHealth,
    /// When an import last succeeded.
    pub last_success_at: Option<DateTime<Utc>>,
    /// When an import was last attempted, successful or not.
    pub last_attempt_at: Option<DateTime<Utc>>,
    /// Stable code of the most recent failure.
    pub last_error_code: Option<String>,
    /// When the runner will try again, if a retry is scheduled.
    pub next_retry_at: Option<DateTime<Utc>>,
    /// Attempts spent on the work that is currently failing.
    pub failed_attempts: u32,
}

impl ConnectionStatus {
    /// Records a successful import at `now`, clearing the failure state.
    pub fn succeeded(&mut self, now: DateTime<Utc>) {
        self.health = ConnectionHealth::Healthy;
        self.last_success_at = Some(now);
        self.last_attempt_at = Some(now);
        self.last_error_code = None;
        self.next_retry_at = None;
        self.failed_attempts = 0;
    }

    /// Records a failed import at `now` and schedules `next_retry_at`.
    ///
    /// Pass `None` for `next_retry_at` when no further attempt is planned,
    /// either because the class is not retryable or because the attempt cap is
    /// spent.
    pub fn failed(
        &mut self,
        now: DateTime<Utc>,
        error: &crate::ConnectorError,
        next_retry_at: Option<DateTime<Utc>>,
    ) {
        self.health = ConnectionHealth::after_failure(error.class);
        self.last_attempt_at = Some(now);
        self.last_error_code = Some(error.code.to_owned());
        self.next_retry_at = next_retry_at;
        self.failed_attempts = self.failed_attempts.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ConnectorError;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_760_000_000, 0).expect("valid timestamp")
    }

    #[test]
    fn retryable_failures_only_degrade_the_connection() {
        for class in [
            RetryClass::RateLimited,
            RetryClass::RetryAfter(Duration::from_secs(30)),
            RetryClass::Unavailable,
            RetryClass::Timeout,
        ] {
            assert_eq!(
                ConnectionHealth::after_failure(class),
                ConnectionHealth::Degraded
            );
            assert!(!ConnectionHealth::after_failure(class).needs_owner_action());
        }
    }

    #[test]
    fn a_revoked_credential_asks_the_owner_to_reconnect() {
        let health = ConnectionHealth::after_failure(RetryClass::Revoked);
        assert_eq!(health, ConnectionHealth::NeedsReconnect);
        assert!(health.needs_owner_action());
        assert_eq!(
            ConnectionHealth::after_failure(RetryClass::Permanent),
            ConnectionHealth::Failed
        );
    }

    #[test]
    fn a_success_clears_the_failure_state() {
        let mut status = ConnectionStatus::default();
        status.failed(now(), &ConnectorError::permanent("boom"), None);
        assert_eq!(status.failed_attempts, 1);

        status.succeeded(now());
        assert_eq!(status.health, ConnectionHealth::Healthy);
        assert_eq!(status.last_error_code, None);
        assert_eq!(status.failed_attempts, 0);
        assert_eq!(status.last_success_at, Some(now()));
    }

    #[test]
    fn failures_accumulate_attempts_and_keep_the_last_code() {
        let mut status = ConnectionStatus::default();
        let error = ConnectorError::new("provider_unavailable", RetryClass::Unavailable);
        status.failed(now(), &error, Some(now()));
        status.failed(now(), &error, Some(now()));

        assert_eq!(status.failed_attempts, 2);
        assert_eq!(status.health, ConnectionHealth::Degraded);
        assert_eq!(
            status.last_error_code.as_deref(),
            Some("provider_unavailable")
        );
        assert_eq!(status.last_success_at, None);
    }

    #[test]
    fn health_serializes_as_a_snake_case_label() {
        let json = serde_json::to_string(&ConnectionHealth::NeedsReconnect).expect("serializes");
        assert_eq!(json, "\"needs_reconnect\"");
        assert_eq!(ConnectionHealth::NeedsReconnect.as_str(), "needs_reconnect");
    }
}
