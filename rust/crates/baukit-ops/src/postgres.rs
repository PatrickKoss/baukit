//! PostgreSQL pool metrics behind the `sqlx-postgres` feature.

use std::{fmt, future::Future, time::Duration};

use baukit_telemetry::metrics;
use sqlx::{PgPool, Postgres, Transaction, pool::PoolConnection};
use tokio::{task::JoinHandle, time::Instant};

const CONNECTIONS_MAX: &str = "db_pool_connections_max";
const CONNECTIONS_IDLE: &str = "db_pool_connections_idle";
const CONNECTIONS_IN_USE: &str = "db_pool_connections_in_use";
const ACQUIRE_DURATION: &str = "db_pool_acquire_duration_seconds";
const ACQUIRE_TIMEOUTS: &str = "db_pool_acquire_timeouts_total";

/// Error returned when a pool-metrics sampler cannot be started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolMetricsSamplerError {
    /// The requested sampling interval was zero.
    ZeroInterval,
}

impl fmt::Display for PoolMetricsSamplerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("database pool metrics interval must be non-zero")
    }
}

impl std::error::Error for PoolMetricsSamplerError {}

/// Handle for a background SQLx PostgreSQL pool-metrics sampler.
///
/// Dropping the handle aborts its task. Use [`PoolMetricsSampler::shutdown`] to
/// abort and await task termination explicitly.
#[derive(Debug)]
pub struct PoolMetricsSampler {
    task: Option<JoinHandle<()>>,
}

impl PoolMetricsSampler {
    /// Aborts the sampler and waits until its task has stopped.
    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    /// Aborts the sampler without waiting for task termination.
    pub fn abort(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Drop for PoolMetricsSampler {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// Spawns a task that periodically samples SQLx PostgreSQL pool gauges.
///
/// The first sample and all zero-valued countable families are emitted before
/// this function returns. The returned handle owns and cancels the task; retain
/// it for as long as sampling should continue.
pub fn spawn_pool_metrics_sampler(
    pool: PgPool,
    interval: Duration,
) -> Result<PoolMetricsSampler, PoolMetricsSamplerError> {
    if interval.is_zero() {
        return Err(PoolMetricsSamplerError::ZeroInterval);
    }

    initialize_pool_metrics();
    sample_pool(&pool);
    let task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            sample_pool(&pool);
        }
    });
    Ok(PoolMetricsSampler { task: Some(task) })
}

/// Acquires a PostgreSQL connection while recording duration and timeouts.
///
/// SQLx does not expose hooks for observing every `PgPool::acquire` call, so
/// applications that require `db_pool_acquire_*` metrics should use this helper
/// at acquisition sites. Direct executor calls on `PgPool` are not included.
pub async fn acquire(pool: &PgPool) -> Result<PoolConnection<Postgres>, sqlx::Error> {
    observe_acquisition(pool.acquire()).await
}

/// Begins a PostgreSQL transaction while recording pool acquisition metrics.
///
/// Use this in place of [`PgPool::begin`]. SQLx combines pool acquisition and
/// the initial `BEGIN` round trip in that API, so the recorded duration includes
/// both. Pool timeouts increment `db_pool_acquire_timeouts_total` just like
/// [`acquire`].
pub async fn begin(pool: &PgPool) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    observe_acquisition(pool.begin()).await
}

async fn observe_acquisition<T>(
    operation: impl Future<Output = Result<T, sqlx::Error>>,
) -> Result<T, sqlx::Error> {
    initialize_pool_metrics();
    let started = Instant::now();
    let result = operation.await;
    metrics::histogram!(ACQUIRE_DURATION).record(started.elapsed().as_secs_f64());
    if matches!(result, Err(sqlx::Error::PoolTimedOut)) {
        metrics::counter!(ACQUIRE_TIMEOUTS).increment(1);
    }
    result
}

fn sample_pool(pool: &PgPool) {
    let size = pool.size();
    let idle = u32::try_from(pool.num_idle()).unwrap_or(u32::MAX);
    let in_use = size.saturating_sub(idle);

    metrics::gauge!(CONNECTIONS_MAX).set(f64::from(pool.options().get_max_connections()));
    metrics::gauge!(CONNECTIONS_IDLE).set(f64::from(idle));
    metrics::gauge!(CONNECTIONS_IN_USE).set(f64::from(in_use));
}

fn initialize_pool_metrics() {
    metrics::describe_gauge!(CONNECTIONS_MAX, "Configured maximum SQLx pool connections");
    metrics::describe_gauge!(CONNECTIONS_IDLE, "Idle SQLx pool connections");
    metrics::describe_gauge!(CONNECTIONS_IN_USE, "In-use SQLx pool connections");
    metrics::describe_histogram!(
        ACQUIRE_DURATION,
        "Time spent acquiring an SQLx pool connection in seconds"
    );
    metrics::describe_counter!(ACQUIRE_TIMEOUTS, "Timed-out SQLx pool acquisitions");
    let _acquire_duration = metrics::histogram!(ACQUIRE_DURATION);
    metrics::counter!(ACQUIRE_TIMEOUTS).absolute(0);
}

#[cfg(test)]
mod tests {
    use metrics_exporter_prometheus::PrometheusBuilder;

    use super::*;

    #[test]
    fn metric_names_match_the_telemetry_contract() {
        assert_eq!(CONNECTIONS_MAX, "db_pool_connections_max");
        assert_eq!(CONNECTIONS_IDLE, "db_pool_connections_idle");
        assert_eq!(CONNECTIONS_IN_USE, "db_pool_connections_in_use");
        assert_eq!(ACQUIRE_DURATION, "db_pool_acquire_duration_seconds");
        assert_eq!(ACQUIRE_TIMEOUTS, "db_pool_acquire_timeouts_total");
    }

    #[tokio::test]
    async fn zero_interval_is_rejected_without_a_database() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").expect("valid database URL");
        assert_eq!(
            spawn_pool_metrics_sampler(pool, Duration::ZERO).expect_err("zero interval"),
            PoolMetricsSamplerError::ZeroInterval
        );
    }

    #[test]
    fn acquire_helper_is_available_without_connecting() {
        let _helper = acquire;
    }

    #[test]
    fn begin_helper_is_available_without_connecting() {
        let _helper = begin;
    }

    #[tokio::test]
    async fn sampler_registers_zero_valued_families_before_the_first_event() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let pool = PgPool::connect_lazy("postgres://localhost/test").expect("valid database URL");

        let sampler = spawn_pool_metrics_sampler(pool, Duration::from_secs(60))
            .expect("valid sampler interval");
        let rendered = handle.render();

        assert!(
            rendered.contains("db_pool_acquire_timeouts_total 0"),
            "{rendered}"
        );
        for gauge in [CONNECTIONS_MAX, CONNECTIONS_IDLE, CONNECTIONS_IN_USE] {
            assert!(rendered.contains(gauge), "missing {gauge} in:\n{rendered}");
        }
        assert!(rendered.contains(ACQUIRE_DURATION), "{rendered}");
        sampler.shutdown().await;
    }
}
