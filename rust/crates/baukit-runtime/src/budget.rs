use std::time::Duration;

use thiserror::Error;

/// The telemetry flush reserve used by [`DrainBudget::new`].
pub const DEFAULT_FLUSH_RESERVE: Duration = Duration::from_secs(3);

/// A drain deadline checked against the request timeout, with a flush reserve.
///
/// The listener drain, supervised tasks and [`ShutdownToken::run_during_drain`]
/// get the drain timeout minus the flush reserve. Only
/// [`ShutdownToken::run_flush`] may use the reserve, so a slow request cannot
/// take the time the telemetry flush needs.
///
/// Build it in the composition root before binding listeners and exit when it
/// fails, so a pod whose grace period cannot fit one request never starts:
///
/// ```
/// use std::time::Duration;
/// use baukit_runtime::{DrainBudget, ShutdownToken};
///
/// let budget = DrainBudget::new(Duration::from_secs(35), Duration::from_secs(30))?;
/// let shutdown = ShutdownToken::with_budget(budget);
/// assert_eq!(shutdown.flush_reserve(), Duration::from_secs(3));
///
/// assert!(DrainBudget::new(Duration::from_secs(25), Duration::from_secs(30)).is_err());
/// # Ok::<(), baukit_runtime::DrainBudgetError>(())
/// ```
///
/// [`ShutdownToken::run_during_drain`]: crate::ShutdownToken::run_during_drain
/// [`ShutdownToken::run_flush`]: crate::ShutdownToken::run_flush
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainBudget {
    drain_timeout: Duration,
    flush_reserve: Duration,
}

impl DrainBudget {
    /// Checks `drain_timeout` against `request_timeout` with the default 3 second flush reserve.
    ///
    /// # Errors
    ///
    /// Fails like [`DrainBudget::with_flush_reserve`].
    pub fn new(
        drain_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, DrainBudgetError> {
        Self::with_flush_reserve(drain_timeout, request_timeout, DEFAULT_FLUSH_RESERVE)
    }

    /// Checks that `request_timeout + flush_reserve < drain_timeout`.
    ///
    /// # Errors
    ///
    /// Returns [`DrainBudgetError`] when one request plus the flush reserve does
    /// not fit in the drain timeout.
    pub fn with_flush_reserve(
        drain_timeout: Duration,
        request_timeout: Duration,
        flush_reserve: Duration,
    ) -> Result<Self, DrainBudgetError> {
        let needed = request_timeout.saturating_add(flush_reserve);
        if needed >= drain_timeout {
            return Err(DrainBudgetError {
                drain_timeout,
                request_timeout,
                flush_reserve,
            });
        }
        Ok(Self {
            drain_timeout,
            flush_reserve,
        })
    }

    /// Returns the total drain timeout.
    pub const fn drain_timeout(&self) -> Duration {
        self.drain_timeout
    }

    /// Returns the part of the drain timeout kept for the telemetry flush.
    pub const fn flush_reserve(&self) -> Duration {
        self.flush_reserve
    }
}

/// The request timeout plus the flush reserve does not fit in the drain timeout.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error(
    "request timeout {request_timeout:?} plus telemetry flush reserve {flush_reserve:?} must be \
     shorter than the drain timeout {drain_timeout:?}; raise the drain timeout (and the pod's \
     termination grace period) or lower the request timeout"
)]
pub struct DrainBudgetError {
    drain_timeout: Duration,
    request_timeout: Duration,
    flush_reserve: Duration,
}

impl DrainBudgetError {
    /// Returns the drain timeout that was too short.
    pub const fn drain_timeout(&self) -> Duration {
        self.drain_timeout
    }

    /// Returns the request timeout that did not fit.
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Returns the flush reserve that was requested.
    pub const fn flush_reserve(&self) -> Duration {
        self.flush_reserve
    }
}
