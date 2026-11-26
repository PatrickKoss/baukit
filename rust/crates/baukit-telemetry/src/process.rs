use std::{
    fs,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Gauge metric for user plus system CPU seconds used by this process.
pub const PROCESS_CPU_SECONDS_TOTAL: &str = "process_cpu_seconds_total";
/// Gauge metric for the resident set size in bytes.
pub const PROCESS_RESIDENT_MEMORY_BYTES: &str = "process_resident_memory_bytes";
/// Gauge metric for open file descriptors.
pub const PROCESS_OPEN_FDS: &str = "process_open_fds";
/// Gauge metric for the soft open file descriptor limit.
pub const PROCESS_MAX_FDS: &str = "process_max_fds";
/// Gauge metric for the Unix time in seconds at which telemetry was initialized.
pub const PROCESS_START_TIME_SECONDS: &str = "process_start_time_seconds";
/// Gauge metric for Tokio runtime worker threads.
#[cfg(feature = "tokio-metrics")]
pub const TOKIO_RUNTIME_WORKERS: &str = "tokio_runtime_workers";
/// Gauge metric for tasks alive on the Tokio runtime.
#[cfg(feature = "tokio-metrics")]
pub const TOKIO_RUNTIME_ALIVE_TASKS: &str = "tokio_runtime_alive_tasks";
/// Gauge metric for tasks waiting in the Tokio runtime's global queue.
#[cfg(feature = "tokio-metrics")]
pub const TOKIO_RUNTIME_GLOBAL_QUEUE_DEPTH: &str = "tokio_runtime_global_queue_depth";
/// Gauge metric for seconds Tokio workers spent busy, summed over all workers.
#[cfg(feature = "tokio-metrics")]
pub const TOKIO_RUNTIME_BUSY_SECONDS_TOTAL: &str = "tokio_runtime_busy_seconds_total";

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const THREAD_NAME: &str = "baukit-process-metrics";
const BYTES_PER_KIB: f64 = 1024.0;
const PROC_SCHEDSTAT: &str = "/proc/self/schedstat";
const PROC_STATUS: &str = "/proc/self/status";
const PROC_LIMITS: &str = "/proc/self/limits";
const PROC_FD_DIR: &str = "/proc/self/fd";
const RSS_PREFIX: &str = "VmRSS:";
const MAX_OPEN_FILES_PREFIX: &str = "Max open files";

/// Refreshes process metrics on a background thread until dropped.
pub(crate) struct ProcessMetricsTask {
    _stop: Option<Sender<()>>,
}

impl ProcessMetricsTask {
    pub(crate) fn start() -> Self {
        record_start_time();
        let sampler = Sampler::capture();
        sampler.record();
        let (stop, stopped) = mpsc::channel::<()>();
        let spawned = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(REFRESH_INTERVAL) {
                    sampler.record();
                }
            });
        match spawned {
            Ok(_) => Self { _stop: Some(stop) },
            Err(error) => {
                tracing::warn!(%error, "process metrics refresh thread could not start");
                Self { _stop: None }
            }
        }
    }
}

struct Sampler {
    #[cfg(feature = "tokio-metrics")]
    runtime: Option<tokio::runtime::Handle>,
}

impl Sampler {
    fn capture() -> Self {
        Self {
            #[cfg(feature = "tokio-metrics")]
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    fn record(&self) {
        record_process_metrics();
        #[cfg(feature = "tokio-metrics")]
        if let Some(runtime) = &self.runtime {
            record_runtime_metrics(runtime);
        }
    }
}

/// Reads `/proc/self` once and updates the `process_*` gauges.
///
/// [`crate::TelemetryBuilder::init`] already refreshes them every 5 seconds.
/// Values that cannot be read, for example on a system without `/proc`, are
/// left unset.
pub fn record_process_metrics() {
    if let Some(seconds) = fs::read_to_string(PROC_SCHEDSTAT)
        .ok()
        .as_deref()
        .and_then(parse_cpu_seconds)
    {
        metrics::gauge!(PROCESS_CPU_SECONDS_TOTAL).set(seconds);
    }
    if let Some(bytes) = fs::read_to_string(PROC_STATUS)
        .ok()
        .as_deref()
        .and_then(parse_resident_bytes)
    {
        metrics::gauge!(PROCESS_RESIDENT_MEMORY_BYTES).set(bytes);
    }
    if let Ok(entries) = fs::read_dir(PROC_FD_DIR) {
        metrics::gauge!(PROCESS_OPEN_FDS).set(count_as_f64(entries.count()));
    }
    if let Some(limit) = fs::read_to_string(PROC_LIMITS)
        .ok()
        .as_deref()
        .and_then(parse_max_open_files)
    {
        metrics::gauge!(PROCESS_MAX_FDS).set(limit);
    }
}

fn record_start_time() {
    if let Ok(since_epoch) = SystemTime::now().duration_since(UNIX_EPOCH) {
        metrics::gauge!(PROCESS_START_TIME_SECONDS).set(since_epoch.as_secs_f64());
    }
}

#[cfg(feature = "tokio-metrics")]
fn record_runtime_metrics(runtime: &tokio::runtime::Handle) {
    let runtime_metrics = runtime.metrics();
    let workers = runtime_metrics.num_workers();
    metrics::gauge!(TOKIO_RUNTIME_WORKERS).set(count_as_f64(workers));
    metrics::gauge!(TOKIO_RUNTIME_ALIVE_TASKS).set(count_as_f64(runtime_metrics.num_alive_tasks()));
    metrics::gauge!(TOKIO_RUNTIME_GLOBAL_QUEUE_DEPTH)
        .set(count_as_f64(runtime_metrics.global_queue_depth()));
    #[cfg(target_has_atomic = "64")]
    {
        let busy: Duration = (0..workers)
            .map(|worker| runtime_metrics.worker_total_busy_duration(worker))
            .sum();
        metrics::gauge!(TOKIO_RUNTIME_BUSY_SECONDS_TOTAL).set(busy.as_secs_f64());
    }
}

fn count_as_f64(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::from(u32::MAX), f64::from)
}

fn parse_cpu_seconds(schedstat: &str) -> Option<f64> {
    let nanos: u64 = schedstat.split_whitespace().next()?.parse().ok()?;
    Some(Duration::from_nanos(nanos).as_secs_f64())
}

fn parse_resident_bytes(status: &str) -> Option<f64> {
    let line = status.lines().find(|line| line.starts_with(RSS_PREFIX))?;
    let kib: u32 = line
        .trim_start_matches(RSS_PREFIX)
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(f64::from(kib) * BYTES_PER_KIB)
}

fn parse_max_open_files(limits: &str) -> Option<f64> {
    let line = limits
        .lines()
        .find(|line| line.starts_with(MAX_OPEN_FILES_PREFIX))?;
    let soft = line
        .trim_start_matches(MAX_OPEN_FILES_PREFIX)
        .split_whitespace()
        .next()?;
    soft.parse::<u32>().ok().map(f64::from)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    use super::*;

    fn gauges(snapshotter: &Snapshotter) -> HashMap<String, f64> {
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| match value {
                DebugValue::Gauge(value) => Some((key.key().name().to_owned(), value.into_inner())),
                _ => None,
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn records_process_metrics_from_proc() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            record_start_time();
            record_process_metrics();
        });

        let gauges = gauges(&snapshotter);
        for name in [
            PROCESS_RESIDENT_MEMORY_BYTES,
            PROCESS_OPEN_FDS,
            PROCESS_MAX_FDS,
            PROCESS_START_TIME_SECONDS,
        ] {
            let value = gauges.get(name).copied().unwrap_or_default();
            assert!(value > 0.0, "{name} = {value}");
        }
        assert!(gauges.contains_key(PROCESS_CPU_SECONDS_TOTAL));
    }

    #[cfg(feature = "tokio-metrics")]
    #[test]
    fn records_stable_tokio_runtime_metrics() {
        const WORKERS: usize = 2;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .build()
            .expect("test runtime");
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || record_runtime_metrics(runtime.handle()));

        let gauges = gauges(&snapshotter);
        assert_eq!(gauges.get(TOKIO_RUNTIME_WORKERS), Some(&2.0));
        assert_eq!(gauges.get(TOKIO_RUNTIME_ALIVE_TASKS), Some(&0.0));
        assert_eq!(gauges.get(TOKIO_RUNTIME_GLOBAL_QUEUE_DEPTH), Some(&0.0));
        assert!(gauges.contains_key(TOKIO_RUNTIME_BUSY_SECONDS_TOTAL));
    }

    #[test]
    fn parses_cpu_seconds_from_schedstat() {
        assert_eq!(parse_cpu_seconds("2500000000 100 7\n"), Some(2.5));
        assert_eq!(parse_cpu_seconds("garbage"), None);
        assert_eq!(parse_cpu_seconds(""), None);
    }

    #[test]
    fn converts_large_cpu_times_without_overflow() {
        assert_eq!(parse_cpu_seconds("5000000000000000"), Some(5_000_000.0));
    }

    #[test]
    fn parses_resident_memory_from_status() {
        let status = "Name:\tapi\nVmPeak:\t  9000 kB\nVmRSS:\t  2048 kB\nThreads:\t4\n";
        assert_eq!(parse_resident_bytes(status), Some(2_097_152.0));
        assert_eq!(parse_resident_bytes("Name:\tapi\n"), None);
    }

    #[test]
    fn parses_the_soft_open_file_limit() {
        let limits = "Limit                     Soft Limit           Hard Limit           Units\n\
                      Max processes             63328                63328                processes\n\
                      Max open files            1024                 1048576              files\n";
        assert_eq!(parse_max_open_files(limits), Some(1024.0));
        let unlimited =
            "Max open files            unlimited            unlimited            files\n";
        assert_eq!(parse_max_open_files(unlimited), None);
    }
}
