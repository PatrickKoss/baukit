use std::time::Duration;

use baukit_telemetry::metrics;

pub(crate) const SUCCESS: &str = "success";
pub(crate) const FAILURE: &str = "failure";
pub(crate) const RETRY: &str = "retry";
const OUTCOMES: [&str; 3] = [SUCCESS, FAILURE, RETRY];
const UNKNOWN_JOB: &str = "unknown";

pub(crate) fn initialize(job_types: &'static [&'static str], queue: &'static str) {
    metrics::describe_counter!("worker_job_runs_total", "Completed worker job attempts");
    metrics::describe_histogram!(
        "worker_job_duration_seconds",
        "Duration of worker job attempts in seconds"
    );
    metrics::describe_gauge!(
        "worker_queue_oldest_age_seconds",
        "Age of the oldest pending job in seconds"
    );

    for job in job_types.iter().copied().chain([UNKNOWN_JOB]) {
        for outcome in OUTCOMES {
            metrics::counter!("worker_job_runs_total", "job" => job, "outcome" => outcome)
                .absolute(0);
        }
        let _histogram = metrics::histogram!("worker_job_duration_seconds", "job" => job);
    }
    metrics::gauge!("worker_queue_oldest_age_seconds", "queue" => queue).set(0.0);
}

pub(crate) fn set_queue_age(queue: &'static str, age: Duration) {
    metrics::gauge!("worker_queue_oldest_age_seconds", "queue" => queue).set(age.as_secs_f64());
}

pub(crate) fn record(
    job_type: &str,
    known_job_types: &'static [&'static str],
    outcome: &'static str,
    duration: Duration,
) {
    let job = known_job_types
        .iter()
        .copied()
        .find(|known| *known == job_type)
        .unwrap_or(UNKNOWN_JOB);
    metrics::counter!("worker_job_runs_total", "job" => job, "outcome" => outcome).increment(1);
    metrics::histogram!("worker_job_duration_seconds", "job" => job).record(duration.as_secs_f64());
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;

    #[test]
    fn worker_metric_names_labels_and_zero_values_match_the_spec() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        initialize(&["sync"], "primary");

        let snapshot = snapshotter.snapshot().into_vec();
        let mut outcomes = BTreeSet::new();
        let mut saw_empty_histogram = false;
        let mut saw_queue_gauge = false;
        for (key, _, _, value) in snapshot {
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect::<Vec<_>>();
            match key.key().name() {
                "worker_job_runs_total" if labels.contains(&("job", "sync")) => {
                    assert_eq!(value, DebugValue::Counter(0));
                    assert_eq!(labels.len(), 2);
                    outcomes.extend(
                        labels
                            .iter()
                            .filter(|(key, _)| *key == "outcome")
                            .map(|(_, value)| (*value).to_owned()),
                    );
                }
                "worker_job_duration_seconds" if labels == [("job", "sync")] => {
                    assert_eq!(value, DebugValue::Histogram(Vec::new()));
                    saw_empty_histogram = true;
                }
                "worker_queue_oldest_age_seconds" if labels == [("queue", "primary")] => {
                    assert_eq!(value, DebugValue::Gauge(0.0.into()));
                    saw_queue_gauge = true;
                }
                _ => {}
            }
        }
        assert_eq!(
            outcomes,
            BTreeSet::from([FAILURE.to_owned(), RETRY.to_owned(), SUCCESS.to_owned()])
        );
        assert!(saw_empty_histogram);
        assert!(saw_queue_gauge);
    }
}
