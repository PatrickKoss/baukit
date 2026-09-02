use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{AmountBudgetStore, RateLimitFailMode};

const UTC_DAY: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_NAMESPACE_LENGTH: usize = 64;

/// Counter for amount-budget decisions, labeled exactly `namespace` and `outcome`.
pub const FIXED_WINDOW_AMOUNT_BUDGET_DECISIONS_TOTAL: &str =
    "fixed_window_amount_budget_decisions_total";

/// Result of checking and conditionally consuming an amount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmountBudgetDecision {
    /// Whether the store consumed the requested amount.
    pub allowed: bool,
    /// Units left in the current window after the decision.
    pub remaining: u64,
    /// UTC instant when the current window ends.
    pub reset_at: SystemTime,
}

/// Port used by callers that consume amount-based budgets.
pub trait AmountBudget: Send + Sync {
    /// Checks and conditionally consumes `amount` for an opaque subject.
    fn consume<'a>(
        &'a self,
        subject: &'a str,
        amount: u64,
    ) -> Pin<Box<dyn Future<Output = AmountBudgetDecision> + Send + 'a>>;
}

/// Clock used to select a fixed window.
pub trait BudgetClock: Send + Sync {
    /// Returns the current UTC instant.
    fn now(&self) -> SystemTime;
}

/// Clock backed by [`SystemTime::now`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBudgetClock;

impl BudgetClock for SystemBudgetClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// An epoch-aligned fixed-window definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedWindow {
    duration: Duration,
}

impl FixedWindow {
    /// Returns a UTC calendar-day window that resets at `00:00:00Z`.
    #[must_use]
    pub const fn utc_day() -> Self {
        Self { duration: UTC_DAY }
    }

    /// Creates a duration window aligned to the Unix epoch.
    pub fn duration(duration: Duration) -> Result<Self, FixedWindowError> {
        if duration.is_zero() {
            return Err(FixedWindowError::ZeroDuration);
        }
        Ok(Self { duration })
    }

    /// Returns the window length.
    #[must_use]
    pub const fn length(self) -> Duration {
        self.duration
    }

    fn at(self, now: SystemTime) -> Option<Window> {
        let now_nanos = epoch_nanos(now)?;
        let duration_nanos = i128::try_from(self.duration.as_nanos()).ok()?;
        let index = now_nanos.div_euclid(duration_nanos);
        let reset_nanos = index.checked_add(1)?.checked_mul(duration_nanos)?;
        Some(Window {
            index,
            reset_at: system_time_from_epoch_nanos(reset_nanos)?,
        })
    }
}

/// Invalid fixed-window definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FixedWindowError {
    /// Duration windows must have a positive length.
    #[error("fixed-window duration must be non-zero")]
    ZeroDuration,
}

/// Validated options for a fixed-window amount budget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixedWindowBudgetOptions {
    namespace: String,
    window: FixedWindow,
    limit: u64,
    fail_mode: RateLimitFailMode,
}

impl FixedWindowBudgetOptions {
    /// Creates options for one configured budget namespace.
    pub fn new(
        namespace: impl Into<String>,
        window: FixedWindow,
        limit: u64,
        fail_mode: RateLimitFailMode,
    ) -> Result<Self, FixedWindowBudgetOptionsError> {
        let namespace = namespace.into();
        if namespace.is_empty() {
            return Err(FixedWindowBudgetOptionsError::EmptyNamespace);
        }
        if namespace.len() > MAX_NAMESPACE_LENGTH {
            return Err(FixedWindowBudgetOptionsError::NamespaceTooLong);
        }
        if !namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(FixedWindowBudgetOptionsError::InvalidNamespace);
        }
        Ok(Self {
            namespace,
            window,
            limit,
            fail_mode,
        })
    }

    /// Returns the metric and key namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the fixed-window definition.
    #[must_use]
    pub const fn window(&self) -> FixedWindow {
        self.window
    }

    /// Returns the maximum units allowed in one window.
    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// Returns the behavior used when the store fails.
    #[must_use]
    pub const fn fail_mode(&self) -> RateLimitFailMode {
        self.fail_mode
    }
}

/// Invalid fixed-window budget options.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FixedWindowBudgetOptionsError {
    /// A namespace is required for keys and bounded metric labels.
    #[error("amount-budget namespace must not be empty")]
    EmptyNamespace,
    /// Namespace values are capped to keep keys and labels small.
    #[error("amount-budget namespace must be at most 64 bytes")]
    NamespaceTooLong,
    /// Namespace values use a restricted ASCII character set.
    #[error("amount-budget namespace may contain only ASCII letters, digits, `_`, `-`, and `.`")]
    InvalidNamespace,
}

/// Fixed-window amount budget backed by an injected store and clock.
#[derive(Clone, Debug)]
pub struct FixedWindowAmountBudget<S, C = SystemBudgetClock> {
    store: S,
    options: FixedWindowBudgetOptions,
    clock: C,
}

impl<S> FixedWindowAmountBudget<S, SystemBudgetClock>
where
    S: AmountBudgetStore,
{
    /// Creates a budget that uses the system clock.
    #[must_use]
    pub fn new(store: S, options: FixedWindowBudgetOptions) -> Self {
        Self::with_clock(store, options, SystemBudgetClock)
    }
}

impl<S, C> FixedWindowAmountBudget<S, C>
where
    S: AmountBudgetStore,
    C: BudgetClock,
{
    /// Creates a budget with an injected clock.
    #[must_use]
    pub fn with_clock(store: S, options: FixedWindowBudgetOptions, clock: C) -> Self {
        metrics::describe_counter!(
            FIXED_WINDOW_AMOUNT_BUDGET_DECISIONS_TOTAL,
            "Fixed-window amount-budget decisions"
        );
        Self {
            store,
            options,
            clock,
        }
    }

    /// Returns the budget options.
    #[must_use]
    pub const fn options(&self) -> &FixedWindowBudgetOptions {
        &self.options
    }
}

impl<S, C> AmountBudget for FixedWindowAmountBudget<S, C>
where
    S: AmountBudgetStore,
    C: BudgetClock,
{
    fn consume<'a>(
        &'a self,
        subject: &'a str,
        amount: u64,
    ) -> Pin<Box<dyn Future<Output = AmountBudgetDecision> + Send + 'a>> {
        Box::pin(async move {
            let now = self.clock.now();
            let Some(window) = self.options.window.at(now) else {
                tracing::warn!(
                    namespace = self.options.namespace(),
                    "amount-budget clock is outside the supported range"
                );
                return self.after_error(now);
            };
            let key = format!(
                "amount-budget:{}:{}:{subject}",
                self.options.namespace, window.index
            );
            match self
                .store
                .check_and_consume_amount(&key, amount, self.options.limit, now, window.reset_at)
                .await
            {
                Ok(decision) => {
                    record(
                        self.options.namespace(),
                        if decision.allowed {
                            "allowed"
                        } else {
                            "denied"
                        },
                    );
                    AmountBudgetDecision {
                        allowed: decision.allowed,
                        remaining: decision.remaining,
                        reset_at: window.reset_at,
                    }
                }
                Err(error) => {
                    record(self.options.namespace(), "error");
                    tracing::warn!(
                        namespace = self.options.namespace(),
                        error = %error,
                        "amount-budget store decision failed"
                    );
                    self.after_error(window.reset_at)
                }
            }
        })
    }
}

impl<S, C> FixedWindowAmountBudget<S, C> {
    fn after_error(&self, reset_at: SystemTime) -> AmountBudgetDecision {
        match self.options.fail_mode {
            RateLimitFailMode::Open => AmountBudgetDecision {
                allowed: true,
                remaining: self.options.limit,
                reset_at,
            },
            RateLimitFailMode::Closed => AmountBudgetDecision {
                allowed: false,
                remaining: 0,
                reset_at,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct Window {
    index: i128,
    reset_at: SystemTime,
}

fn epoch_nanos(time: SystemTime) -> Option<i128> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_nanos()).ok(),
        Err(error) => i128::try_from(error.duration().as_nanos())
            .ok()
            .and_then(i128::checked_neg),
    }
}

fn system_time_from_epoch_nanos(nanos: i128) -> Option<SystemTime> {
    let magnitude = nanos.unsigned_abs();
    let seconds = u64::try_from(magnitude / 1_000_000_000).ok()?;
    let subsecond_nanos = u32::try_from(magnitude % 1_000_000_000).ok()?;
    let duration = Duration::new(seconds, subsecond_nanos);
    if nanos >= 0 {
        UNIX_EPOCH.checked_add(duration)
    } else {
        UNIX_EPOCH.checked_sub(duration)
    }
}

fn record(namespace: &str, outcome: &'static str) {
    metrics::counter!(
        FIXED_WINDOW_AMOUNT_BUDGET_DECISIONS_TOTAL,
        "namespace" => namespace.to_owned(),
        "outcome" => outcome
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{AmountBudgetStoreDecision, InMemoryRateLimitStore, RateLimitStoreError};

    use super::*;

    #[derive(Clone)]
    struct TestClock(Arc<Mutex<SystemTime>>);

    impl TestClock {
        fn new(now: SystemTime) -> Self {
            Self(Arc::new(Mutex::new(now)))
        }

        fn set(&self, now: SystemTime) {
            *self.0.lock().expect("clock") = now;
        }
    }

    impl BudgetClock for TestClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().expect("clock")
        }
    }

    #[tokio::test]
    async fn injected_clock_expires_usage_at_the_duration_boundary() {
        let store = InMemoryRateLimitStore::new(1).expect("store");
        let clock = TestClock::new(UNIX_EPOCH + Duration::from_secs(1_005));
        let options = FixedWindowBudgetOptions::new(
            "uploads",
            FixedWindow::duration(Duration::from_secs(10)).expect("window"),
            10,
            RateLimitFailMode::Closed,
        )
        .expect("options");
        let budget = FixedWindowAmountBudget::with_clock(store, options, clock.clone());

        let first = budget.consume("subject", 10).await;
        assert!(first.allowed);
        assert_eq!(first.remaining, 0);
        assert_eq!(first.reset_at, UNIX_EPOCH + Duration::from_secs(1_010));

        clock.set(UNIX_EPOCH + Duration::from_secs(1_010));
        let next = budget.consume("subject", 1).await;
        assert!(next.allowed);
        assert_eq!(next.remaining, 9);
        assert_eq!(next.reset_at, UNIX_EPOCH + Duration::from_secs(1_020));
    }

    #[test]
    fn utc_day_resets_at_the_next_midnight() {
        let before_midnight = UNIX_EPOCH + Duration::from_secs(1_788_393_599);
        let midnight = UNIX_EPOCH + Duration::from_secs(1_788_393_600);
        let window = FixedWindow::utc_day().at(before_midnight).expect("window");
        assert_eq!(window.reset_at, midnight);

        let next = FixedWindow::utc_day().at(midnight).expect("window");
        assert_eq!(next.reset_at, midnight + UTC_DAY);
    }

    #[tokio::test]
    async fn store_error_obeys_open_and_closed_modes() {
        let now = UNIX_EPOCH + Duration::from_secs(1_005);
        let clock = TestClock::new(now);
        let window = FixedWindow::duration(Duration::from_secs(10)).expect("window");
        let open = FixedWindowAmountBudget::with_clock(
            FailingStore,
            FixedWindowBudgetOptions::new("open", window, 10, RateLimitFailMode::Open)
                .expect("options"),
            clock.clone(),
        );
        let closed = FixedWindowAmountBudget::with_clock(
            FailingStore,
            FixedWindowBudgetOptions::new("closed", window, 10, RateLimitFailMode::Closed)
                .expect("options"),
            clock,
        );

        assert_eq!(
            open.consume("subject", 4).await,
            AmountBudgetDecision {
                allowed: true,
                remaining: 10,
                reset_at: UNIX_EPOCH + Duration::from_secs(1_010),
            }
        );
        assert_eq!(
            closed.consume("subject", 4).await,
            AmountBudgetDecision {
                allowed: false,
                remaining: 0,
                reset_at: UNIX_EPOCH + Duration::from_secs(1_010),
            }
        );
    }

    #[test]
    fn rejects_zero_duration_and_unbounded_namespace_values() {
        assert_eq!(
            FixedWindow::duration(Duration::ZERO),
            Err(FixedWindowError::ZeroDuration)
        );
        assert_eq!(
            FixedWindowBudgetOptions::new(
                "subject:value",
                FixedWindow::utc_day(),
                10,
                RateLimitFailMode::Open,
            ),
            Err(FixedWindowBudgetOptionsError::InvalidNamespace)
        );
    }

    #[derive(Clone, Copy)]
    struct FailingStore;

    impl AmountBudgetStore for FailingStore {
        fn check_and_consume_amount<'a>(
            &'a self,
            _key: &'a str,
            _amount: u64,
            _limit: u64,
            _now: SystemTime,
            _reset_at: SystemTime,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<AmountBudgetStoreDecision, RateLimitStoreError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(RateLimitStoreError::unavailable("fixture unavailable")) })
        }
    }
}
