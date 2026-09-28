//! Deferred receipt polling for notifications the provider accepted but had not settled.

use std::{collections::BTreeMap, num::NonZeroU32};

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;

use crate::{
    DeviceRegistry, DeviceToken, PushDeliveryStatus, PushError, PushOutcome, PushReceiptSource,
    PushStoreError, PushStoreFuture, PushTicketId,
};

/// How long a recorded ticket waits before its first poll, and between polls.
///
/// Expo recommends checking receipts 15 minutes after the send.
pub const RECEIPT_POLL_DELAY: TimeDelta = TimeDelta::minutes(15);

/// How long the provider keeps a receipt after the send.
///
/// Expo clears receipts after 24 hours. A ticket older than this never
/// settles; [`PendingReceiptStore::purge`] removes it.
pub const RECEIPT_RETENTION: TimeDelta = TimeDelta::hours(24);

/// An accepted notification whose receipt has not been read yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingReceipt {
    /// The ticket the provider issued for the notification.
    pub ticket: PushTicketId,
    /// The device token the notification went to.
    pub token: DeviceToken,
    /// The instant the send started; invalidation compares it.
    pub sent_at: DateTime<Utc>,
}

/// Storage port for tickets whose receipts are still outstanding.
///
/// Each ticket carries a due instant. [`PendingReceiptStore::take_due`] moves
/// the tickets it returns to a later due instant, so concurrent pollers take
/// disjoint batches and a ticket that stays unsettled goes to the back of the
/// queue instead of blocking newer ones.
pub trait PendingReceiptStore: Send + Sync {
    /// Records tickets first due at `due_at`; returns how many were new.
    ///
    /// A ticket that is already recorded is left unchanged.
    fn record(
        &self,
        receipts: Vec<PendingReceipt>,
        due_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>>;

    /// Takes up to `limit` tickets due at or before `now`, oldest due first,
    /// and makes them due again at `retry_at`.
    fn take_due(
        &self,
        now: DateTime<Utc>,
        retry_at: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> PushStoreFuture<'_, Result<Vec<PendingReceipt>, PushStoreError>>;

    /// Deletes tickets whose receipts settled; returns how many went.
    fn delete(
        &self,
        tickets: Vec<PushTicketId>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>>;

    /// Deletes up to `limit` tickets sent before `sent_before`; returns how many went.
    ///
    /// Pass `now - RECEIPT_RETENTION`. Call it again until it returns less
    /// than `limit`.
    fn purge(
        &self,
        sent_before: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>>;

    /// Records every [`PushDeliveryStatus::Accepted`] outcome of one send.
    ///
    /// Call it after [`PushSender::send`](crate::PushSender::send) with the
    /// instant the send started. The tickets are first due
    /// [`RECEIPT_POLL_DELAY`] later. Outcomes without an accepted ticket cost
    /// no store round trip.
    fn record_accepted(
        &self,
        outcomes: &[PushOutcome],
        sent_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let receipts = accepted_receipts(outcomes, sent_at);
        if receipts.is_empty() {
            return Box::pin(std::future::ready(Ok(0)));
        }
        self.record(receipts, sent_at + RECEIPT_POLL_DELAY)
    }
}

/// Returns a pending receipt for each accepted outcome of one send.
#[must_use]
pub fn accepted_receipts(outcomes: &[PushOutcome], sent_at: DateTime<Utc>) -> Vec<PendingReceipt> {
    outcomes
        .iter()
        .filter_map(|outcome| match &outcome.status {
            PushDeliveryStatus::Accepted(ticket) => Some(PendingReceipt {
                ticket: ticket.clone(),
                token: outcome.token.clone(),
                sent_at,
            }),
            PushDeliveryStatus::Delivered | PushDeliveryStatus::Rejected(_) => None,
        })
        .collect()
}

/// What one [`poll_pending_receipts`] run did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReceiptPoll {
    /// Tickets taken from the store. Fewer than the limit means the queue is drained.
    pub taken: u64,
    /// Tickets whose receipt settled and that were deleted.
    pub settled: u64,
    /// Device tokens removed from the registry.
    pub invalidated: u64,
}

/// A failure that stopped a receipt poll.
///
/// Taken tickets stay recorded and come due again after [`RECEIPT_POLL_DELAY`].
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReceiptPollError {
    /// The pending receipt store or the registry failed.
    #[error(transparent)]
    Store(#[from] PushStoreError),
    /// The provider's receipt request failed.
    #[error(transparent)]
    Push(#[from] PushError),
}

/// Polls one batch of due receipts and applies what settled.
///
/// Takes up to `limit` due tickets, fetches their receipts, invalidates every
/// `DeviceNotRegistered` token with the `sent_at` of its own send, and deletes
/// the settled tickets. Tickets without a receipt yet stay recorded for a
/// later run. Invalidation runs before deletion, so a failure in between
/// repeats the invalidation on the next run, which removes nothing new.
///
/// # Errors
///
/// Returns [`ReceiptPollError`] when the store, the registry, or the receipt
/// request fails.
pub async fn poll_pending_receipts<S, P, R>(
    source: &S,
    pending: &P,
    registry: &R,
    now: DateTime<Utc>,
    limit: NonZeroU32,
) -> Result<ReceiptPoll, ReceiptPollError>
where
    S: PushReceiptSource + ?Sized,
    P: PendingReceiptStore + ?Sized,
    R: DeviceRegistry + ?Sized,
{
    let due = pending
        .take_due(now, now + RECEIPT_POLL_DELAY, limit)
        .await?;
    let taken = count(due.len());
    if due.is_empty() {
        return Ok(ReceiptPoll::default());
    }
    let receipts = source
        .receipts(due.iter().map(|receipt| receipt.ticket.clone()).collect())
        .await?;

    let mut dead_by_send = BTreeMap::<DateTime<Utc>, Vec<DeviceToken>>::new();
    let mut settled = Vec::new();
    for pending_receipt in due {
        let Some(receipt) = receipts.get(&pending_receipt.ticket) else {
            continue;
        };
        if receipt.is_token_dead() {
            dead_by_send
                .entry(pending_receipt.sent_at)
                .or_default()
                .push(pending_receipt.token);
        }
        settled.push(pending_receipt.ticket);
    }

    let mut invalidated = 0;
    for (sent_at, mut tokens) in dead_by_send {
        tokens.sort();
        tokens.dedup();
        invalidated += registry.invalidate(tokens, sent_at).await?;
    }
    let settled = if settled.is_empty() {
        0
    } else {
        pending.delete(settled).await?
    };
    Ok(ReceiptPoll {
        taken,
        settled,
        invalidated,
    })
}

fn count(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        DevicePlatform, DeviceRegistration, FakePushSender, MemoryDeviceRegistry,
        MemoryPendingReceiptStore, PushMessage, PushReceipt, PushRejection, PushSender,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const LIMIT: NonZeroU32 = NonZeroU32::new(10).expect("ten is not zero");

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    fn token(value: &str) -> DeviceToken {
        DeviceToken::new(value).expect("valid test token")
    }

    async fn register(registry: &MemoryDeviceRegistry, owner_id: Uuid, value: &str, minutes: i64) {
        registry
            .register(DeviceRegistration::new(
                owner_id,
                token(value),
                DevicePlatform::Ios,
                at(minutes),
            ))
            .await
            .expect("memory register");
    }

    async fn registered(registry: &MemoryDeviceRegistry, owner_id: Uuid) -> Vec<String> {
        let mut tokens = registry
            .list_for_owner(owner_id)
            .await
            .expect("memory list")
            .into_iter()
            .map(|device| device.token.expose().to_owned())
            .collect::<Vec<_>>();
        tokens.sort();
        tokens
    }

    #[tokio::test]
    async fn a_late_device_not_registered_receipt_removes_the_token() -> TestResult {
        let sender = FakePushSender::new();
        let pending = MemoryPendingReceiptStore::new();
        let registry = MemoryDeviceRegistry::new();
        let owner = Uuid::now_v7();
        let names = ["fine", "gone", "reinstalled", "slow"];
        for name in names {
            register(&registry, owner, name, 0).await;
            sender.accept_without_receipt(token(name)).await;
        }

        let sent_at = at(1);
        let outcomes = sender
            .send(
                names
                    .iter()
                    .map(|name| PushMessage::new(token(name), "t", "b"))
                    .collect(),
            )
            .await?;
        assert_eq!(pending.record_accepted(&outcomes, sent_at).await?, 4);
        assert_eq!(
            poll_pending_receipts(&sender, &pending, &registry, at(5), LIMIT).await?,
            ReceiptPoll::default(),
            "nothing is due before the poll delay"
        );

        let dead = PushReceipt::Rejected(PushRejection::DeviceNotRegistered);
        sender
            .settle_receipt(token("fine"), PushReceipt::Delivered)
            .await;
        sender.settle_receipt(token("gone"), dead.clone()).await;
        sender.settle_receipt(token("reinstalled"), dead).await;
        register(&registry, owner, "reinstalled", 10).await;

        let due_at = sent_at + RECEIPT_POLL_DELAY;
        let poll = poll_pending_receipts(&sender, &pending, &registry, due_at, LIMIT).await?;
        assert_eq!(
            poll,
            ReceiptPoll {
                taken: 4,
                settled: 3,
                invalidated: 1
            }
        );
        assert_eq!(
            registered(&registry, owner).await,
            ["fine", "reinstalled", "slow"],
            "a registration after the send outlives the receipt"
        );
        let still_pending = pending.pending();
        assert_eq!(still_pending.len(), 1);
        assert_eq!(still_pending[0].token, token("slow"));

        assert_eq!(
            poll_pending_receipts(&sender, &pending, &registry, due_at, LIMIT)
                .await?
                .taken,
            0,
            "a still-pending ticket waits for the next delay"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_receipt_request_keeps_the_tickets() -> TestResult {
        let sender = FakePushSender::new();
        let pending = MemoryPendingReceiptStore::new();
        let registry = MemoryDeviceRegistry::new();
        sender.accept_without_receipt(token("slow")).await;
        let outcomes = sender
            .send(vec![PushMessage::new(token("slow"), "t", "b")])
            .await?;
        pending.record_accepted(&outcomes, at(0)).await?;

        sender
            .fail_with(PushError::Transport {
                class: baukit_http::RetryClass::Unavailable,
            })
            .await;
        let error = poll_pending_receipts(&sender, &pending, &registry, at(20), LIMIT)
            .await
            .expect_err("the receipt request fails");
        assert!(matches!(error, ReceiptPollError::Push(_)));
        assert_eq!(pending.pending().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn settled_outcomes_record_nothing() -> TestResult {
        let pending = MemoryPendingReceiptStore::new();
        let outcomes = [PushOutcome {
            token: token("fine"),
            status: PushDeliveryStatus::Delivered,
        }];
        assert_eq!(pending.record_accepted(&outcomes, at(0)).await?, 0);
        assert!(accepted_receipts(&outcomes, at(0)).is_empty());
        Ok(())
    }
}
