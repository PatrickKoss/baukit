//! In-memory [`PushSender`] for tests, behind the `test-support` feature.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::Mutex;

use crate::{
    PushDeliveryStatus, PushError, PushFuture, PushMessage, PushOutcome, PushRejection, PushSender,
};

#[derive(Default)]
struct State {
    batches: Vec<Vec<PushMessage>>,
    outcomes: Vec<PushOutcome>,
    rejections: HashMap<String, PushRejection>,
    accepted: HashSet<String>,
    failure: Option<PushError>,
}

/// Recording [`PushSender`] that answers from a scripted table instead of a network.
///
/// Every token delivers by default. Script exceptions per token with
/// [`FakePushSender::reject`] and [`FakePushSender::accept_without_receipt`],
/// or fail the whole batch with [`FakePushSender::fail_with`]. Clones share one
/// recording, so a clone handed to a service under test still reports what that
/// service sent.
#[derive(Clone, Default)]
pub struct FakePushSender {
    state: Arc<Mutex<State>>,
}

impl FakePushSender {
    /// Creates a sender that delivers every notification.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes one token reject with the given reason on every send.
    ///
    /// Pass [`PushRejection::DeviceNotRegistered`] to exercise token pruning.
    pub async fn reject(&self, token: impl Into<String>, rejection: PushRejection) {
        self.state
            .lock()
            .await
            .rejections
            .insert(token.into(), rejection);
    }

    /// Makes one token report [`PushDeliveryStatus::Accepted`] with no receipt.
    pub async fn accept_without_receipt(&self, token: impl Into<String>) {
        self.state.lock().await.accepted.insert(token.into());
    }

    /// Makes every subsequent send fail the whole batch with this error.
    pub async fn fail_with(&self, error: PushError) {
        self.state.lock().await.failure = Some(error);
    }

    /// Clears a previously scripted whole-batch failure.
    pub async fn clear_failure(&self) {
        self.state.lock().await.failure = None;
    }

    /// Returns the batches passed to [`PushSender::send`], in call order.
    pub async fn batches(&self) -> Vec<Vec<PushMessage>> {
        self.state.lock().await.batches.clone()
    }

    /// Returns every message sent so far, flattened across batches.
    pub async fn messages(&self) -> Vec<PushMessage> {
        self.state
            .lock()
            .await
            .batches
            .iter()
            .flatten()
            .cloned()
            .collect()
    }

    /// Returns every outcome this sender has reported.
    pub async fn outcomes(&self) -> Vec<PushOutcome> {
        self.state.lock().await.outcomes.clone()
    }

    /// Returns the tokens reported as dead, ready to prune.
    pub async fn dead_tokens(&self) -> Vec<String> {
        self.state
            .lock()
            .await
            .outcomes
            .iter()
            .filter(|outcome| outcome.is_token_dead())
            .map(|outcome| outcome.token.clone())
            .collect()
    }
}

impl PushSender for FakePushSender {
    fn send<'a>(&'a self, batch: Vec<PushMessage>) -> PushFuture<'a> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            if let Some(failure) = state.failure.clone() {
                return Err(failure);
            }
            let outcomes = batch
                .iter()
                .map(|message| PushOutcome {
                    token: message.token.clone(),
                    status: match state.rejections.get(&message.token) {
                        Some(rejection) => PushDeliveryStatus::Rejected(rejection.clone()),
                        None if state.accepted.contains(&message.token) => {
                            PushDeliveryStatus::Accepted
                        }
                        None => PushDeliveryStatus::Delivered,
                    },
                })
                .collect::<Vec<_>>();
            state.batches.push(batch);
            state.outcomes.extend(outcomes.clone());
            Ok(outcomes)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_token_delivers_and_every_batch_is_recorded() -> Result<(), PushError> {
        let sender = FakePushSender::new();
        let outcomes = sender
            .send(vec![
                PushMessage::new("a", "t", "b"),
                PushMessage::new("b", "t", "b"),
            ])
            .await?;
        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.status == PushDeliveryStatus::Delivered)
        );
        assert_eq!(sender.batches().await.len(), 1);
        assert_eq!(sender.messages().await.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn scripted_tokens_reject_and_surface_as_dead() -> Result<(), PushError> {
        let sender = FakePushSender::new();
        sender
            .reject("gone", PushRejection::DeviceNotRegistered)
            .await;
        sender.reject("big", PushRejection::MessageTooBig).await;
        sender.accept_without_receipt("slow").await;

        let outcomes = sender
            .send(
                ["gone", "big", "slow", "fine"]
                    .into_iter()
                    .map(|token| PushMessage::new(token, "t", "b"))
                    .collect(),
            )
            .await?;
        let by_token = outcomes
            .into_iter()
            .map(|outcome| (outcome.token, outcome.status))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            by_token["gone"],
            PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered)
        );
        assert_eq!(
            by_token["big"],
            PushDeliveryStatus::Rejected(PushRejection::MessageTooBig)
        );
        assert_eq!(by_token["slow"], PushDeliveryStatus::Accepted);
        assert_eq!(by_token["fine"], PushDeliveryStatus::Delivered);
        assert_eq!(sender.dead_tokens().await, vec!["gone".to_owned()]);
        Ok(())
    }

    #[tokio::test]
    async fn a_scripted_failure_applies_until_it_is_cleared() -> Result<(), PushError> {
        let sender = FakePushSender::new();
        sender
            .fail_with(PushError::Transport {
                class: baukit_http::RetryClass::Unavailable,
            })
            .await;
        let error = sender
            .send(vec![PushMessage::new("a", "t", "b")])
            .await
            .expect_err("scripted failure");
        assert!(error.is_retryable());
        assert!(sender.batches().await.is_empty());

        sender.clear_failure().await;
        assert_eq!(
            sender
                .send(vec![PushMessage::new("a", "t", "b")])
                .await?
                .len(),
            1
        );
        Ok(())
    }
}
