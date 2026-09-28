//! In-memory [`PushSender`] and [`PushReceiptSource`] for tests, behind the `test-support` feature.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::Mutex;

use crate::{
    DeviceToken, PushDeliveryStatus, PushError, PushFuture, PushMessage, PushOutcome, PushReceipt,
    PushReceiptFuture, PushReceiptSource, PushRejection, PushSender, PushTicketId,
};

#[derive(Default)]
struct State {
    batches: Vec<Vec<PushMessage>>,
    outcomes: Vec<PushOutcome>,
    rejections: HashMap<DeviceToken, PushRejection>,
    accepted: HashSet<DeviceToken>,
    tickets: HashMap<PushTicketId, DeviceToken>,
    receipts: HashMap<DeviceToken, PushReceipt>,
    receipt_requests: Vec<Vec<PushTicketId>>,
    failure: Option<PushError>,
}

impl State {
    fn status(&mut self, token: &DeviceToken) -> PushDeliveryStatus {
        if let Some(rejection) = self.rejections.get(token) {
            return PushDeliveryStatus::Rejected(rejection.clone());
        }
        if !self.accepted.contains(token) {
            return PushDeliveryStatus::Delivered;
        }
        let ticket = PushTicketId::new(format!("fake-ticket-{}", self.tickets.len()))
            .expect("fake ticket ids are valid");
        self.tickets.insert(ticket.clone(), token.clone());
        PushDeliveryStatus::Accepted(ticket)
    }
}

/// Recording [`PushSender`] and [`PushReceiptSource`] that answers from a
/// scripted table instead of a network.
///
/// Every token delivers by default. Script exceptions per token with
/// [`FakePushSender::reject`] and [`FakePushSender::accept_without_receipt`],
/// settle an accepted token's later receipt with
/// [`FakePushSender::settle_receipt`], or fail every send and receipt request
/// with [`FakePushSender::fail_with`]. Clones share one recording, so a clone
/// handed to a service under test still reports what that service sent.
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
    pub async fn reject(&self, token: DeviceToken, rejection: PushRejection) {
        self.state.lock().await.rejections.insert(token, rejection);
    }

    /// Makes one token report [`PushDeliveryStatus::Accepted`] with a fresh ticket.
    pub async fn accept_without_receipt(&self, token: DeviceToken) {
        self.state.lock().await.accepted.insert(token);
    }

    /// Makes receipt requests report `receipt` for every ticket addressed to `token`.
    ///
    /// Tickets of a token without a settled receipt stay absent from
    /// [`PushReceiptSource::receipts`].
    pub async fn settle_receipt(&self, token: DeviceToken, receipt: PushReceipt) {
        self.state.lock().await.receipts.insert(token, receipt);
    }

    /// Makes every subsequent send and receipt request fail with this error.
    pub async fn fail_with(&self, error: PushError) {
        self.state.lock().await.failure = Some(error);
    }

    /// Clears a previously scripted failure.
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
    pub async fn dead_tokens(&self) -> Vec<DeviceToken> {
        self.state
            .lock()
            .await
            .outcomes
            .iter()
            .filter(|outcome| outcome.is_token_dead())
            .map(|outcome| outcome.token.clone())
            .collect()
    }

    /// Returns the ticket IDs passed to [`PushReceiptSource::receipts`], in call order.
    pub async fn receipt_requests(&self) -> Vec<Vec<PushTicketId>> {
        self.state.lock().await.receipt_requests.clone()
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
                    status: state.status(&message.token),
                })
                .collect::<Vec<_>>();
            state.batches.push(batch);
            state.outcomes.extend(outcomes.clone());
            Ok(outcomes)
        })
    }
}

impl PushReceiptSource for FakePushSender {
    fn receipts<'a>(&'a self, tickets: Vec<PushTicketId>) -> PushReceiptFuture<'a> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            if let Some(failure) = state.failure.clone() {
                return Err(failure);
            }
            let settled = tickets
                .iter()
                .filter_map(|ticket| {
                    let token = state.tickets.get(ticket)?;
                    let receipt = state.receipts.get(token)?;
                    Some((ticket.clone(), receipt.clone()))
                })
                .collect();
            state.receipt_requests.push(tickets);
            Ok(settled)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> DeviceToken {
        DeviceToken::new(value).expect("valid test token")
    }

    fn message(value: &str) -> PushMessage {
        PushMessage::new(token(value), "t", "b")
    }

    #[tokio::test]
    async fn every_token_delivers_and_every_batch_is_recorded() -> Result<(), PushError> {
        let sender = FakePushSender::new();
        let outcomes = sender.send(vec![message("a"), message("b")]).await?;
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
            .reject(token("gone"), PushRejection::DeviceNotRegistered)
            .await;
        sender
            .reject(token("big"), PushRejection::MessageTooBig)
            .await;
        sender.accept_without_receipt(token("slow")).await;

        let outcomes = sender
            .send(
                ["gone", "big", "slow", "fine"]
                    .into_iter()
                    .map(message)
                    .collect(),
            )
            .await?;
        let by_token = outcomes
            .into_iter()
            .map(|outcome| (outcome.token.expose().to_owned(), outcome.status))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            by_token["gone"],
            PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered)
        );
        assert_eq!(
            by_token["big"],
            PushDeliveryStatus::Rejected(PushRejection::MessageTooBig)
        );
        assert!(matches!(by_token["slow"], PushDeliveryStatus::Accepted(_)));
        assert_eq!(by_token["fine"], PushDeliveryStatus::Delivered);
        assert_eq!(sender.dead_tokens().await, vec![token("gone")]);
        Ok(())
    }

    #[tokio::test]
    async fn an_accepted_ticket_settles_once_its_receipt_is_scripted() -> Result<(), PushError> {
        let sender = FakePushSender::new();
        sender.accept_without_receipt(token("slow")).await;
        let outcomes = sender.send(vec![message("slow")]).await?;
        let PushDeliveryStatus::Accepted(ticket) = outcomes[0].status.clone() else {
            panic!("the token was scripted as accepted");
        };

        assert!(sender.receipts(vec![ticket.clone()]).await?.is_empty());
        sender
            .settle_receipt(
                token("slow"),
                PushReceipt::Rejected(PushRejection::DeviceNotRegistered),
            )
            .await;
        let receipts = sender.receipts(vec![ticket.clone()]).await?;
        assert!(receipts[&ticket].is_token_dead());
        assert_eq!(sender.receipt_requests().await.len(), 2);
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
            .send(vec![message("a")])
            .await
            .expect_err("scripted failure");
        assert!(error.is_retryable());
        assert!(sender.batches().await.is_empty());
        assert!(sender.receipts(Vec::new()).await.is_err());

        sender.clear_failure().await;
        assert_eq!(sender.send(vec![message("a")]).await?.len(), 1);
        Ok(())
    }
}
