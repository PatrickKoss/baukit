//! In-memory registry, claim, and pending receipt stores for tests, behind the `test-support` feature.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::{
    DEFAULT_DEVICES_PER_OWNER, DeliveryClaim, DeliveryClaimStore, DeviceRegistration,
    DeviceRegistry, DeviceToken, PendingReceipt, PendingReceiptStore, PushStoreError,
    PushStoreFuture, PushTicketId, RegisteredDevice, RegistrationOutcome,
};

/// [`DeviceRegistry`] held in memory with the same cap, move, and eviction
/// rules as the PostgreSQL adapter.
///
/// Clones share one set of devices, so a clone handed to a service under test
/// still shows what that service registered.
#[derive(Clone, Debug)]
pub struct MemoryDeviceRegistry {
    devices: Arc<Mutex<HashMap<DeviceToken, RegisteredDevice>>>,
    devices_per_owner: NonZeroU32,
}

impl Default for MemoryDeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryDeviceRegistry {
    /// Creates an empty registry that keeps [`DEFAULT_DEVICES_PER_OWNER`] devices per owner.
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Arc::default(),
            devices_per_owner: DEFAULT_DEVICES_PER_OWNER,
        }
    }

    /// Sets how many devices each owner keeps before the oldest are evicted.
    #[must_use]
    pub const fn with_devices_per_owner(mut self, maximum: NonZeroU32) -> Self {
        self.devices_per_owner = maximum;
        self
    }

    fn devices(&self) -> MutexGuard<'_, HashMap<DeviceToken, RegisteredDevice>> {
        self.devices.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn register_device(
        &self,
        previous: Option<DeviceToken>,
        registration: DeviceRegistration,
    ) -> RegistrationOutcome {
        let mut devices = self.devices();
        if let Some(previous) = previous.filter(|previous| *previous != registration.token)
            && devices
                .get(&previous)
                .is_some_and(|device| device.owner_id == registration.owner_id)
        {
            devices.remove(&previous);
        }
        let device = merge(devices.get(&registration.token), registration);
        let owner_id = device.owner_id;
        let kept = device.token.clone();
        devices.insert(kept.clone(), device);
        let evicted = evict_over_cap(&mut devices, owner_id, &kept, self.devices_per_owner);
        RegistrationOutcome { evicted }
    }
}

fn merge(
    existing: Option<&RegisteredDevice>,
    registration: DeviceRegistration,
) -> RegisteredDevice {
    let same_owner = existing.filter(|device| device.owner_id == registration.owner_id);
    RegisteredDevice {
        owner_id: registration.owner_id,
        token: registration.token,
        platform: registration.platform,
        time_zone: registration.time_zone,
        created_at: same_owner.map_or(registration.registered_at, |device| device.created_at),
        last_registered_at: same_owner.map_or(registration.registered_at, |device| {
            device.last_registered_at.max(registration.registered_at)
        }),
    }
}

fn evict_over_cap(
    devices: &mut HashMap<DeviceToken, RegisteredDevice>,
    owner_id: Uuid,
    kept: &DeviceToken,
    devices_per_owner: NonZeroU32,
) -> u64 {
    let mut others = devices
        .values()
        .filter(|device| device.owner_id == owner_id && device.token != *kept)
        .collect::<Vec<_>>();
    others.sort_by_key(|device| Reverse(recency(device)));
    let others_kept = usize::try_from(devices_per_owner.get() - 1).unwrap_or(usize::MAX);
    let evicted = others
        .into_iter()
        .skip(others_kept)
        .map(|device| device.token.clone())
        .collect::<Vec<_>>();
    for token in &evicted {
        devices.remove(token);
    }
    u64::try_from(evicted.len()).unwrap_or(u64::MAX)
}

fn recency(device: &RegisteredDevice) -> (DateTime<Utc>, DateTime<Utc>, &DeviceToken) {
    (device.last_registered_at, device.created_at, &device.token)
}

fn count(removed: usize) -> u64 {
    u64::try_from(removed).unwrap_or(u64::MAX)
}

impl DeviceRegistry for MemoryDeviceRegistry {
    fn register(
        &self,
        registration: DeviceRegistration,
    ) -> PushStoreFuture<'_, Result<RegistrationOutcome, PushStoreError>> {
        let outcome = self.register_device(None, registration);
        Box::pin(std::future::ready(Ok(outcome)))
    }

    fn rotate(
        &self,
        previous: DeviceToken,
        registration: DeviceRegistration,
    ) -> PushStoreFuture<'_, Result<RegistrationOutcome, PushStoreError>> {
        let outcome = self.register_device(Some(previous), registration);
        Box::pin(std::future::ready(Ok(outcome)))
    }

    fn unregister(
        &self,
        owner_id: Uuid,
        token: DeviceToken,
    ) -> PushStoreFuture<'_, Result<bool, PushStoreError>> {
        let mut devices = self.devices();
        let owned = devices
            .get(&token)
            .is_some_and(|device| device.owner_id == owner_id);
        if owned {
            devices.remove(&token);
        }
        Box::pin(std::future::ready(Ok(owned)))
    }

    fn list_for_owner(
        &self,
        owner_id: Uuid,
    ) -> PushStoreFuture<'_, Result<Vec<RegisteredDevice>, PushStoreError>> {
        let mut owned = self
            .devices()
            .values()
            .filter(|device| device.owner_id == owner_id)
            .cloned()
            .collect::<Vec<_>>();
        owned.sort_by(|left, right| recency(right).cmp(&recency(left)));
        Box::pin(std::future::ready(Ok(owned)))
    }

    fn invalidate(
        &self,
        tokens: Vec<DeviceToken>,
        sent_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let dead = tokens.into_iter().collect::<HashSet<_>>();
        let mut devices = self.devices();
        let before = devices.len();
        devices.retain(|token, device| {
            !(dead.contains(token) && device.last_registered_at <= sent_at)
        });
        let removed = count(before - devices.len());
        Box::pin(std::future::ready(Ok(removed)))
    }

    fn erase_owner(&self, owner_id: Uuid) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let mut devices = self.devices();
        let before = devices.len();
        devices.retain(|_, device| device.owner_id != owner_id);
        let removed = count(before - devices.len());
        Box::pin(std::future::ready(Ok(removed)))
    }
}

/// [`DeliveryClaimStore`] held in memory. Clones share one set of claims.
#[derive(Clone, Debug, Default)]
pub struct MemoryDeliveryClaimStore {
    claims: Arc<Mutex<HashSet<DeliveryClaim>>>,
}

impl MemoryDeliveryClaimStore {
    /// Creates a store with no claims.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether the claim is currently held.
    #[must_use]
    pub fn is_claimed(&self, claim: &DeliveryClaim) -> bool {
        self.claims().contains(claim)
    }

    fn claims(&self) -> MutexGuard<'_, HashSet<DeliveryClaim>> {
        self.claims.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl DeliveryClaimStore for MemoryDeliveryClaimStore {
    fn claim(
        &self,
        claim: DeliveryClaim,
        _claimed_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<bool, PushStoreError>> {
        let inserted = self.claims().insert(claim);
        Box::pin(std::future::ready(Ok(inserted)))
    }

    fn release(&self, claim: DeliveryClaim) -> PushStoreFuture<'_, Result<bool, PushStoreError>> {
        let removed = self.claims().remove(&claim);
        Box::pin(std::future::ready(Ok(removed)))
    }

    fn purge(
        &self,
        before: NaiveDate,
        limit: NonZeroU32,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let mut claims = self.claims();
        let mut expired = claims
            .iter()
            .filter(|claim| claim.local_date < before)
            .map(|claim| (claim.local_date, claim.owner_id, claim.kind.clone()))
            .collect::<Vec<_>>();
        expired.sort();
        let removed = expired
            .into_iter()
            .take(limit_len(limit))
            .filter(|(local_date, owner_id, kind)| {
                claims.remove(&DeliveryClaim::new(*owner_id, *local_date, kind.clone()))
            })
            .count();
        Box::pin(std::future::ready(Ok(count(removed))))
    }
}

#[derive(Clone, Debug)]
struct DueReceipt {
    receipt: PendingReceipt,
    due_at: DateTime<Utc>,
}

/// [`PendingReceiptStore`] held in memory with the PostgreSQL adapter's
/// ordering. Clones share one set of tickets.
#[derive(Clone, Debug, Default)]
pub struct MemoryPendingReceiptStore {
    receipts: Arc<Mutex<HashMap<PushTicketId, DueReceipt>>>,
}

impl MemoryPendingReceiptStore {
    /// Creates a store with no tickets.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns every recorded ticket, ordered by ticket ID.
    #[must_use]
    pub fn pending(&self) -> Vec<PendingReceipt> {
        let mut pending = self
            .receipts()
            .values()
            .map(|due| due.receipt.clone())
            .collect::<Vec<_>>();
        pending.sort_by(|left, right| left.ticket.cmp(&right.ticket));
        pending
    }

    fn receipts(&self) -> MutexGuard<'_, HashMap<PushTicketId, DueReceipt>> {
        self.receipts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn limit_len(limit: NonZeroU32) -> usize {
    usize::try_from(limit.get()).unwrap_or(usize::MAX)
}

impl PendingReceiptStore for MemoryPendingReceiptStore {
    fn record(
        &self,
        receipts: Vec<PendingReceipt>,
        due_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let mut stored = self.receipts();
        let before = stored.len();
        for receipt in receipts {
            stored
                .entry(receipt.ticket.clone())
                .or_insert(DueReceipt { receipt, due_at });
        }
        let recorded = count(stored.len() - before);
        Box::pin(std::future::ready(Ok(recorded)))
    }

    fn take_due(
        &self,
        now: DateTime<Utc>,
        retry_at: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> PushStoreFuture<'_, Result<Vec<PendingReceipt>, PushStoreError>> {
        let mut stored = self.receipts();
        let mut due = stored
            .values()
            .filter(|due| due.due_at <= now)
            .map(|due| (due.due_at, due.receipt.ticket.clone()))
            .collect::<Vec<_>>();
        due.sort();
        let mut taken = Vec::new();
        for (_, ticket) in due.into_iter().take(limit_len(limit)) {
            if let Some(due) = stored.get_mut(&ticket) {
                due.due_at = retry_at;
                taken.push(due.receipt.clone());
            }
        }
        Box::pin(std::future::ready(Ok(taken)))
    }

    fn delete(
        &self,
        tickets: Vec<PushTicketId>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let mut stored = self.receipts();
        let removed = tickets
            .iter()
            .filter(|ticket| stored.remove(*ticket).is_some())
            .count();
        Box::pin(std::future::ready(Ok(count(removed))))
    }

    fn purge(
        &self,
        sent_before: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let mut stored = self.receipts();
        let mut expired = stored
            .values()
            .filter(|due| due.receipt.sent_at < sent_before)
            .map(|due| (due.receipt.sent_at, due.receipt.ticket.clone()))
            .collect::<Vec<_>>();
        expired.sort();
        let removed = expired
            .into_iter()
            .take(limit_len(limit))
            .filter(|(_, ticket)| stored.remove(ticket).is_some())
            .count();
        Box::pin(std::future::ready(Ok(count(removed))))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeDelta};

    use super::*;
    use crate::{DeliveryKind, DevicePlatform, PushDeliveryStatus, PushOutcome, PushRejection};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const CAP: NonZeroU32 = NonZeroU32::new(2).expect("two is not zero");

    fn token(value: &str) -> DeviceToken {
        DeviceToken::new(value).expect("valid test token")
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    fn registration(owner_id: Uuid, value: &str, minutes: i64) -> DeviceRegistration {
        DeviceRegistration::new(owner_id, token(value), DevicePlatform::Android, at(minutes))
    }

    async fn tokens(registry: &MemoryDeviceRegistry, owner_id: Uuid) -> Vec<String> {
        registry
            .list_for_owner(owner_id)
            .await
            .expect("memory list")
            .iter()
            .map(|device| device.token.expose().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn the_cap_evicts_the_least_recently_registered_device() -> TestResult {
        let registry = MemoryDeviceRegistry::new().with_devices_per_owner(CAP);
        let owner = Uuid::now_v7();
        registry.register(registration(owner, "a", 1)).await?;
        registry.register(registration(owner, "b", 2)).await?;
        registry.register(registration(owner, "a", 3)).await?;
        let outcome = registry.register(registration(owner, "c", 4)).await?;
        assert_eq!(outcome.evicted, 1);
        assert_eq!(tokens(&registry, owner).await, ["c", "a"]);
        Ok(())
    }

    #[tokio::test]
    async fn rotation_replaces_the_predecessor_without_evicting() -> TestResult {
        let registry = MemoryDeviceRegistry::new().with_devices_per_owner(CAP);
        let owner = Uuid::now_v7();
        registry.register(registration(owner, "a", 1)).await?;
        registry.register(registration(owner, "b", 2)).await?;
        let outcome = registry
            .rotate(token("a"), registration(owner, "a2", 3))
            .await?;
        assert_eq!(outcome.evicted, 0);
        assert_eq!(tokens(&registry, owner).await, ["a2", "b"]);
        Ok(())
    }

    #[tokio::test]
    async fn a_token_moves_to_the_owner_who_registered_it_last() -> TestResult {
        let registry = MemoryDeviceRegistry::new();
        let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
        registry.register(registration(first, "shared", 1)).await?;
        registry.register(registration(second, "shared", 2)).await?;
        assert!(tokens(&registry, first).await.is_empty());
        assert_eq!(tokens(&registry, second).await, ["shared"]);
        assert!(!registry.unregister(first, token("shared")).await?);
        Ok(())
    }

    #[tokio::test]
    async fn dead_outcomes_remove_only_tokens_registered_before_the_send() -> TestResult {
        let registry = MemoryDeviceRegistry::new();
        let owner = Uuid::now_v7();
        registry.register(registration(owner, "old", 1)).await?;
        registry.register(registration(owner, "fresh", 5)).await?;
        let dead = |value: &str| PushOutcome {
            token: token(value),
            status: PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered),
        };
        let removed = registry
            .invalidate_dead_tokens(&[dead("old"), dead("fresh")], at(3))
            .await?;
        assert_eq!(removed, 1);
        assert_eq!(tokens(&registry, owner).await, ["fresh"]);
        assert_eq!(registry.erase_owner(owner).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_claim_is_held_once_until_released() -> TestResult {
        let store = MemoryDeliveryClaimStore::new();
        let claim = DeliveryClaim::new(
            Uuid::now_v7(),
            NaiveDate::from_ymd_opt(2026, 9, 27).expect("valid date"),
            DeliveryKind::new("daily_reminder")?,
        );
        assert!(store.claim(claim.clone(), at(0)).await?);
        assert!(!store.claim(claim.clone(), at(1)).await?);
        assert!(store.release(claim.clone()).await?);
        assert!(!store.is_claimed(&claim));
        assert!(store.claim(claim, at(2)).await?);
        Ok(())
    }

    #[tokio::test]
    async fn claims_before_the_cutoff_purge_oldest_first_in_batches() -> TestResult {
        let store = MemoryDeliveryClaimStore::new();
        let owner = Uuid::now_v7();
        let kind = DeliveryKind::new("daily_reminder")?;
        let claim = |day| {
            DeliveryClaim::new(
                owner,
                NaiveDate::from_ymd_opt(2026, 9, day).expect("valid date"),
                kind.clone(),
            )
        };
        for day in [24, 25, 26, 27] {
            store.claim(claim(day), at(0)).await?;
        }
        let cutoff = claim(27).local_date;
        let one = NonZeroU32::MIN;
        assert_eq!(store.purge(cutoff, one).await?, 1);
        assert!(!store.is_claimed(&claim(24)));
        assert!(store.is_claimed(&claim(25)));
        assert_eq!(store.purge(cutoff, CAP).await?, 2);
        assert_eq!(store.purge(cutoff, CAP).await?, 0);
        assert!(store.is_claimed(&claim(27)));
        Ok(())
    }

    fn pending(ticket: &str, sent_minutes: i64) -> PendingReceipt {
        PendingReceipt {
            ticket: PushTicketId::new(ticket).expect("valid test ticket"),
            token: token(&format!("token-{ticket}")),
            sent_at: at(sent_minutes),
        }
    }

    fn tickets(receipts: &[PendingReceipt]) -> Vec<&str> {
        receipts
            .iter()
            .map(|receipt| receipt.ticket.as_str())
            .collect()
    }

    #[tokio::test]
    async fn due_tickets_are_taken_oldest_first_and_move_back() -> TestResult {
        let store = MemoryPendingReceiptStore::new();
        store
            .record(vec![pending("b", 0), pending("a", 0)], at(10))
            .await?;
        store.record(vec![pending("c", 5)], at(15)).await?;
        assert_eq!(store.record(vec![pending("a", 0)], at(99)).await?, 0);

        assert!(store.take_due(at(9), at(30), CAP).await?.is_empty());
        let first = store.take_due(at(20), at(30), CAP).await?;
        assert_eq!(tickets(&first), ["a", "b"]);
        let second = store.take_due(at(20), at(30), CAP).await?;
        assert_eq!(tickets(&second), ["c"]);
        let retried = store.take_due(at(30), at(45), CAP).await?;
        assert_eq!(tickets(&retried), ["a", "b"]);
        Ok(())
    }

    #[tokio::test]
    async fn settled_and_expired_tickets_are_deleted() -> TestResult {
        let store = MemoryPendingReceiptStore::new();
        store
            .record(
                vec![pending("a", 0), pending("b", 1), pending("c", 2)],
                at(0),
            )
            .await?;
        let a = PushTicketId::new("a")?;
        assert_eq!(store.delete(vec![a.clone(), a]).await?, 1);
        assert_eq!(store.purge(at(2), CAP).await?, 1);
        assert_eq!(tickets(&store.pending()), ["c"]);
        Ok(())
    }
}
