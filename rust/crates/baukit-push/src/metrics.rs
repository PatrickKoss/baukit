//! Send counters. Labels are static outcome names, never tokens or owners.

use crate::{PushDeliveryStatus, PushOutcome, PushRejection};

/// Counter of notifications by `outcome`, one sample per notification per send attempt.
pub const PUSH_NOTIFICATIONS_TOTAL: &str = "push_notifications_total";

const DELIVERED: &str = "delivered";
const ACCEPTED: &str = "accepted";
const TOKEN_DEAD: &str = "token_dead";
const REJECTED: &str = "rejected";
const FAILED: &str = "failed";
const OUTCOMES: [&str; 5] = [DELIVERED, ACCEPTED, TOKEN_DEAD, REJECTED, FAILED];

pub(crate) fn describe() {
    metrics::describe_counter!(
        PUSH_NOTIFICATIONS_TOTAL,
        "Push notifications by send outcome"
    );
    for outcome in OUTCOMES {
        metrics::counter!(PUSH_NOTIFICATIONS_TOTAL, "outcome" => outcome).increment(0);
    }
}

pub(crate) fn record_outcomes(outcomes: &[PushOutcome]) {
    for outcome in outcomes {
        metrics::counter!(PUSH_NOTIFICATIONS_TOTAL, "outcome" => outcome_label(&outcome.status))
            .increment(1);
    }
}

pub(crate) fn record_failed(count: usize) {
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    metrics::counter!(PUSH_NOTIFICATIONS_TOTAL, "outcome" => FAILED).increment(count);
}

const fn outcome_label(status: &PushDeliveryStatus) -> &'static str {
    match status {
        PushDeliveryStatus::Delivered => DELIVERED,
        PushDeliveryStatus::Accepted(_) => ACCEPTED,
        PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered) => TOKEN_DEAD,
        PushDeliveryStatus::Rejected(_) => REJECTED,
    }
}

#[cfg(test)]
mod tests {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;
    use crate::{DeviceToken, PushTicketId};

    fn outcome(status: PushDeliveryStatus) -> Result<PushOutcome, Box<dyn std::error::Error>> {
        Ok(PushOutcome {
            token: DeviceToken::new("ExponentPushToken[secret-token]")?,
            status,
        })
    }

    #[test]
    fn sends_count_by_outcome_without_token_labels() -> Result<(), Box<dyn std::error::Error>> {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let outcomes = vec![
            outcome(PushDeliveryStatus::Delivered)?,
            outcome(PushDeliveryStatus::Accepted(PushTicketId::new("ticket-1")?))?,
            outcome(PushDeliveryStatus::Rejected(
                PushRejection::DeviceNotRegistered,
            ))?,
            outcome(PushDeliveryStatus::Rejected(PushRejection::Other(
                "x".into(),
            )))?,
        ];
        metrics::with_local_recorder(&recorder, || {
            describe();
            record_outcomes(&outcomes);
            record_failed(3);
        });
        let mut counts = Vec::new();
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            let labels = key.key().labels().collect::<Vec<_>>();
            assert_eq!(key.key().name(), PUSH_NOTIFICATIONS_TOTAL);
            assert_eq!(labels.len(), 1);
            let [label] = labels.as_slice() else {
                continue;
            };
            assert_eq!(label.key(), "outcome");
            let DebugValue::Counter(count) = value else {
                return Err("not a counter".into());
            };
            counts.push((label.value().to_owned(), count));
        }
        counts.sort();
        assert_eq!(
            counts,
            vec![
                (ACCEPTED.to_owned(), 1),
                (DELIVERED.to_owned(), 1),
                (FAILED.to_owned(), 3),
                (REJECTED.to_owned(), 1),
                (TOKEN_DEAD.to_owned(), 1),
            ]
        );
        Ok(())
    }
}
