use std::{collections::BTreeSet, error::Error, num::NonZeroU32, path::PathBuf};

use baukit_push::{
    DeliveryClaim, DeliveryClaimStore, DeliveryKind, DevicePlatform, DeviceRegistration,
    DeviceRegistry, DeviceTimeZone, DeviceToken, FakePushSender, MAX_DEVICE_TOKEN_LENGTH,
    MAX_PUSH_TICKET_ID_LENGTH, PendingReceipt, PendingReceiptStore, PostgresDeliveryClaimStore,
    PostgresDeviceRegistry, PostgresPendingReceiptStore, PushDeliveryStatus, PushMessage,
    PushOutcome, PushReceipt, PushRejection, PushSender, PushTicketId, RECEIPT_POLL_DELAY,
    RECEIPT_RETENTION, ReceiptPoll, erase_owner_delivery_claims, erase_owner_push_devices,
    poll_pending_receipts, purge_delivery_claims,
};
use baukit_test::PostgresTestContainer;
use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone as _, Utc};
use sqlx::PgPool;
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

const CAP: NonZeroU32 = NonZeroU32::new(3).expect("three is not zero");
const CONCURRENT_REGISTRATIONS: usize = 16;
const CONCURRENT_OWNERS: usize = 8;
const CONCURRENT_CLAIMS: usize = 10;
const PURGE_BATCH: NonZeroU32 = NonZeroU32::new(1).expect("one is not zero");
const CONCURRENT_POLLERS: usize = 6;
const POLL_LIMIT: NonZeroU32 = NonZeroU32::new(10).expect("ten is not zero");

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0)
        .single()
        .expect("valid test instant")
        + TimeDelta::minutes(minutes)
}

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, day).expect("valid test date")
}

fn token(value: &str) -> DeviceToken {
    DeviceToken::new(value).expect("valid test token")
}

fn registration(owner_id: Uuid, value: &str, minutes: i64) -> DeviceRegistration {
    DeviceRegistration::new(owner_id, token(value), DevicePlatform::Ios, at(minutes))
}

fn dead(value: &str) -> PushOutcome {
    PushOutcome {
        token: token(value),
        status: PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered),
    }
}

async fn fixture() -> Result<(PostgresTestContainer, PgPool), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(
        "CREATE TABLE owners (id UUID PRIMARY KEY);
         ALTER TABLE push_devices
             ADD CONSTRAINT push_devices_owner_fk
             FOREIGN KEY (owner_id) REFERENCES owners (id) ON DELETE CASCADE;
         ALTER TABLE push_delivery_claims
             ADD CONSTRAINT push_delivery_claims_owner_fk
             FOREIGN KEY (owner_id) REFERENCES owners (id) ON DELETE CASCADE;",
    )
    .execute(&pool)
    .await?;
    Ok((fixture, pool))
}

async fn owner(pool: &PgPool) -> Result<Uuid, TestError> {
    let owner_id = Uuid::now_v7();
    sqlx::query("INSERT INTO owners (id) VALUES ($1)")
        .bind(owner_id)
        .execute(pool)
        .await?;
    Ok(owner_id)
}

async fn tokens(
    registry: &PostgresDeviceRegistry,
    owner_id: Uuid,
) -> Result<Vec<String>, TestError> {
    Ok(registry
        .list_for_owner(owner_id)
        .await?
        .iter()
        .map(|device| device.token.expose().to_owned())
        .collect())
}

async fn device_count(pool: &PgPool) -> Result<i64, TestError> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM push_devices")
        .fetch_one(pool)
        .await?)
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_device_registers_refreshes_and_unregisters_for_its_owner_only() -> Result<(), TestError>
{
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone());
    let (alice, bob) = (owner(&pool).await?, owner(&pool).await?);
    let longest = "t".repeat(MAX_DEVICE_TOKEN_LENGTH);

    registry
        .register(registration(alice, &longest, 1).with_time_zone(DeviceTimeZone::new("UTC")?))
        .await?;
    registry
        .register(
            DeviceRegistration::new(alice, token(&longest), DevicePlatform::Android, at(5))
                .with_time_zone(DeviceTimeZone::new("Europe/Berlin")?),
        )
        .await?;
    registry.register(registration(alice, "stale", 3)).await?;

    let devices = registry.list_for_owner(alice).await?;
    assert_eq!(devices.len(), 2);
    let refreshed = &devices[0];
    assert_eq!(refreshed.token.expose(), longest);
    assert_eq!(refreshed.platform, DevicePlatform::Android);
    assert_eq!(
        refreshed.time_zone.as_ref().map(DeviceTimeZone::as_str),
        Some("Europe/Berlin")
    );
    assert_eq!(refreshed.created_at, at(1));
    assert_eq!(refreshed.last_registered_at, at(5));

    registry.register(registration(alice, "stale", 2)).await?;
    let stale = &registry.list_for_owner(alice).await?[1];
    assert_eq!(
        stale.last_registered_at,
        at(3),
        "a late refresh never moves time back"
    );

    assert!(!registry.unregister(bob, token("stale")).await?);
    assert!(registry.unregister(alice, token("stale")).await?);
    assert!(!registry.unregister(alice, token("stale")).await?);
    assert_eq!(tokens(&registry, alice).await?, [longest]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn the_cap_evicts_the_oldest_device_deterministically() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone()).with_devices_per_owner(CAP);
    let (alice, bob) = (owner(&pool).await?, owner(&pool).await?);
    registry.register(registration(bob, "bob", 0)).await?;

    registry.register(registration(alice, "a", 1)).await?;
    registry.register(registration(alice, "c", 2)).await?;
    registry.register(registration(alice, "b", 2)).await?;
    registry.register(registration(alice, "a", 3)).await?;
    let outcome = registry.register(registration(alice, "d", 4)).await?;
    assert_eq!(outcome.evicted, 1);
    assert_eq!(
        tokens(&registry, alice).await?,
        ["d", "a", "c"],
        "equal instants evict the lower token first"
    );

    let skewed = registry.register(registration(alice, "e", -10)).await?;
    assert_eq!(skewed.evicted, 1);
    assert_eq!(
        tokens(&registry, alice).await?,
        ["d", "a", "e"],
        "the device being registered is never evicted, even with an older clock"
    );
    assert_eq!(tokens(&registry, bob).await?, ["bob"]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_registrations_never_overshoot_the_cap() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone()).with_devices_per_owner(CAP);
    let alice = owner(&pool).await?;

    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..CONCURRENT_REGISTRATIONS {
        let registry = registry.clone();
        tasks.spawn(async move {
            let minutes = i64::try_from(index).expect("small index");
            registry
                .register(registration(alice, &format!("device-{index}"), minutes))
                .await
        });
    }
    let mut evicted = 0;
    while let Some(outcome) = tasks.join_next().await {
        evicted += outcome??.evicted;
    }

    let remaining = registry.list_for_owner(alice).await?.len();
    assert_eq!(remaining, usize::try_from(CAP.get())?);
    assert_eq!(
        evicted,
        u64::try_from(CONCURRENT_REGISTRATIONS - remaining)?
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn rotation_replaces_the_predecessor_and_survives_a_race() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone()).with_devices_per_owner(CAP);
    let alice = owner(&pool).await?;
    for (value, minutes) in [("a", 1), ("b", 2), ("c", 3)] {
        registry
            .register(registration(alice, value, minutes))
            .await?;
    }

    let at_cap = registry
        .rotate(token("a"), registration(alice, "a2", 4))
        .await?;
    assert_eq!(
        at_cap.evicted, 0,
        "rotating at the cap keeps every other device"
    );
    assert_eq!(tokens(&registry, alice).await?, ["a2", "c", "b"]);

    let (left, right) = tokio::join!(
        registry.rotate(token("a2"), registration(alice, "left", 5)),
        registry.rotate(token("a2"), registration(alice, "right", 5)),
    );
    left?;
    right?;
    let after_race = tokens(&registry, alice).await?;
    assert!(!after_race.contains(&"a2".to_owned()));
    assert!(after_race.contains(&"left".to_owned()) && after_race.contains(&"right".to_owned()));
    assert_eq!(after_race.len(), usize::try_from(CAP.get())?);

    let retried = registry
        .rotate(token("a2"), registration(alice, "right", 6))
        .await?;
    assert_eq!(retried.evicted, 0, "a retried rotation is a refresh");
    assert_eq!(tokens(&registry, alice).await?, after_race);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_shared_token_ends_with_exactly_one_owner() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone());
    let mut owners = Vec::with_capacity(CONCURRENT_OWNERS);
    for _ in 0..CONCURRENT_OWNERS {
        owners.push(owner(&pool).await?);
    }

    let mut tasks = tokio::task::JoinSet::new();
    for (index, owner_id) in owners.iter().copied().enumerate() {
        let registry = registry.clone();
        tasks.spawn(async move {
            let minutes = i64::try_from(index).expect("small index");
            registry
                .register(registration(owner_id, "shared-device", minutes))
                .await
        });
    }
    while let Some(outcome) = tasks.join_next().await {
        outcome??;
    }

    assert_eq!(device_count(&pool).await?, 1);
    let mut holders = BTreeSet::new();
    for owner_id in &owners {
        if !registry.list_for_owner(*owner_id).await?.is_empty() {
            holders.insert(*owner_id);
        }
    }
    assert_eq!(holders.len(), 1);

    let holder = *holders.first().expect("one holder");
    let newcomer = owners
        .iter()
        .copied()
        .find(|owner_id| *owner_id != holder)
        .expect("a second owner");
    registry
        .register(registration(newcomer, "shared-device", 100))
        .await?;
    let moved = registry.list_for_owner(newcomer).await?;
    assert_eq!(moved[0].created_at, at(100), "a moved token starts fresh");
    assert!(registry.list_for_owner(holder).await?.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_dead_token_receipt_removes_the_token_in_one_call() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone());
    let (alice, bob) = (owner(&pool).await?, owner(&pool).await?);
    registry.register(registration(alice, "gone", 1)).await?;
    registry.register(registration(alice, "alive", 1)).await?;
    registry
        .register(registration(bob, "reinstalled", 1))
        .await?;

    let sent_at = at(5);
    registry
        .register(registration(bob, "reinstalled", 6))
        .await?;
    let removed = registry
        .invalidate_dead_tokens(
            &[
                dead("gone"),
                dead("gone"),
                dead("reinstalled"),
                dead("never-registered"),
                PushOutcome {
                    token: token("alive"),
                    status: PushDeliveryStatus::Rejected(PushRejection::MessageTooBig),
                },
            ],
            sent_at,
        )
        .await?;

    assert_eq!(removed, 1);
    assert_eq!(tokens(&registry, alice).await?, ["alive"]);
    assert_eq!(
        tokens(&registry, bob).await?,
        ["reinstalled"],
        "a registration after the send outlives the receipt"
    );
    assert_eq!(
        registry
            .invalidate_dead_tokens(&[dead("gone")], sent_at)
            .await?,
        0
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn account_erasure_removes_every_device_and_claim() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone());
    let claims = PostgresDeliveryClaimStore::new(pool.clone());
    let kind = DeliveryKind::new("daily_reminder")?;
    let (alice, bob, carol) = (
        owner(&pool).await?,
        owner(&pool).await?,
        owner(&pool).await?,
    );
    for owner_id in [alice, bob, carol] {
        for device in ["phone", "tablet"] {
            registry
                .register(registration(owner_id, &format!("{owner_id}-{device}"), 1))
                .await?;
        }
        claims
            .claim(DeliveryClaim::new(owner_id, date(27), kind.clone()), at(0))
            .await?;
    }

    assert_eq!(registry.erase_owner(alice).await?, 2);
    assert!(registry.list_for_owner(alice).await?.is_empty());

    let mut transaction = pool.begin().await?;
    assert_eq!(erase_owner_push_devices(&mut *transaction, bob).await?, 2);
    assert_eq!(
        erase_owner_delivery_claims(&mut *transaction, bob).await?,
        1
    );
    transaction.commit().await?;

    sqlx::query("DELETE FROM owners WHERE id = $1")
        .bind(carol)
        .execute(&pool)
        .await?;
    assert_eq!(device_count(&pool).await?, 0);
    let remaining_claims: i64 = sqlx::query_scalar("SELECT count(*) FROM push_delivery_claims")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        remaining_claims, 1,
        "only the port-erased owner's claim is left"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_daily_claim_has_exactly_one_winner_until_released() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let claims = PostgresDeliveryClaimStore::new(pool.clone());
    let alice = owner(&pool).await?;
    let claim = DeliveryClaim::new(alice, date(27), DeliveryKind::new("training_reminder")?);

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..CONCURRENT_CLAIMS {
        let claims = claims.clone();
        let claim = claim.clone();
        tasks.spawn(async move { claims.claim(claim, at(0)).await });
    }
    let mut winners = 0;
    while let Some(won) = tasks.join_next().await {
        winners += usize::from(won??);
    }
    assert_eq!(winners, 1);

    let other_kind = DeliveryClaim::new(alice, date(27), DeliveryKind::new("weekly_summary")?);
    let next_day = DeliveryClaim::new(alice, date(28), claim.kind.clone());
    assert!(claims.claim(other_kind, at(0)).await?);
    assert!(claims.claim(next_day, at(0)).await?);

    assert!(claims.release(claim.clone()).await?);
    assert!(!claims.release(claim.clone()).await?);
    assert!(claims.claim(claim, at(1)).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn old_claims_purge_in_bounded_batches() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let claims = PostgresDeliveryClaimStore::new(pool.clone());
    let alice = owner(&pool).await?;
    let kind = DeliveryKind::new("daily_reminder")?;
    for day in [24, 25, 26, 27] {
        claims
            .claim(DeliveryClaim::new(alice, date(day), kind.clone()), at(0))
            .await?;
    }

    let mut purged = Vec::new();
    loop {
        let deleted = purge_delivery_claims(&pool, date(26), PURGE_BATCH).await?;
        purged.push(deleted);
        if deleted < u64::from(PURGE_BATCH.get()) {
            break;
        }
    }
    assert_eq!(purged, [1, 1, 0]);
    let kept: Vec<NaiveDate> =
        sqlx::query_scalar("SELECT local_date FROM push_delivery_claims ORDER BY local_date")
            .fetch_all(&pool)
            .await?;
    assert_eq!(kept, [date(26), date(27)]);
    Ok(())
}

fn ticket(value: &str) -> PushTicketId {
    PushTicketId::new(value).expect("valid test ticket")
}

fn pending(ticket_id: &str, token_value: &str, sent_minutes: i64) -> PendingReceipt {
    PendingReceipt {
        ticket: ticket(ticket_id),
        token: token(token_value),
        sent_at: at(sent_minutes),
    }
}

fn ticket_ids(receipts: &[PendingReceipt]) -> Vec<String> {
    receipts
        .iter()
        .map(|receipt| receipt.ticket.as_str().to_owned())
        .collect()
}

async fn stored_tickets(pool: &PgPool) -> Result<Vec<String>, TestError> {
    Ok(
        sqlx::query_scalar("SELECT ticket_id FROM push_pending_receipts ORDER BY ticket_id")
            .fetch_all(pool)
            .await?,
    )
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn pending_receipts_round_trip_and_come_due_again_after_a_take() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let store = PostgresPendingReceiptStore::new(pool.clone());
    let longest_ticket = "t".repeat(MAX_PUSH_TICKET_ID_LENGTH);
    let longest_token = "d".repeat(MAX_DEVICE_TOKEN_LENGTH);

    let recorded = store
        .record(
            vec![
                pending(&longest_ticket, &longest_token, 0),
                pending("b", "token-b", 0),
                pending("b", "token-b", 0),
            ],
            at(15),
        )
        .await?;
    assert_eq!(recorded, 2);
    assert_eq!(
        store.record(vec![pending("b", "other", 9)], at(99)).await?,
        0,
        "a recorded ticket is left unchanged"
    );
    store
        .record(vec![pending("c", "token-c", 5)], at(20))
        .await?;

    assert!(store.take_due(at(14), at(30), POLL_LIMIT).await?.is_empty());
    let first = store.take_due(at(15), at(30), POLL_LIMIT).await?;
    let mut first_ids = ticket_ids(&first);
    first_ids.sort();
    assert_eq!(first_ids, ["b".to_owned(), longest_ticket.clone()]);
    let longest = first
        .iter()
        .find(|receipt| receipt.ticket.as_str() == longest_ticket)
        .expect("the longest ticket was taken");
    assert_eq!(longest.token.expose(), longest_token);
    assert_eq!(longest.sent_at, at(0));

    assert_eq!(
        ticket_ids(&store.take_due(at(25), at(40), POLL_LIMIT).await?),
        ["c"],
        "taken tickets wait for their retry instant"
    );
    assert_eq!(store.take_due(at(30), at(45), POLL_LIMIT).await?.len(), 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_pollers_take_disjoint_batches() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let store = PostgresPendingReceiptStore::new(pool.clone());
    let tickets = (0..CONCURRENT_POLLERS * 2)
        .map(|index| pending(&format!("ticket-{index:02}"), &format!("token-{index}"), 0))
        .collect::<Vec<_>>();
    store.record(tickets, at(0)).await?;

    let batch = NonZeroU32::new(2).expect("two is not zero");
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..CONCURRENT_POLLERS {
        let store = store.clone();
        tasks.spawn(async move { store.take_due(at(1), at(30), batch).await });
    }
    let mut taken = Vec::new();
    while let Some(batch) = tasks.join_next().await {
        taken.extend(ticket_ids(&batch??));
    }
    let distinct = taken.iter().collect::<BTreeSet<_>>();
    assert_eq!(distinct.len(), taken.len(), "no ticket was taken twice");
    assert_eq!(taken.len(), CONCURRENT_POLLERS * 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn settled_and_expired_receipts_are_deleted() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let store = PostgresPendingReceiptStore::new(pool.clone());
    store
        .record(
            vec![
                pending("a", "token-a", 0),
                pending("b", "token-b", 1),
                pending("c", "token-c", 2),
                pending("d", "token-d", 3),
            ],
            at(0),
        )
        .await?;

    assert_eq!(store.delete(vec![ticket("a"), ticket("unknown")]).await?, 1);
    assert_eq!(store.delete(Vec::new()).await?, 0);

    let mut purged = Vec::new();
    loop {
        let deleted = store.purge(at(3), PURGE_BATCH).await?;
        purged.push(deleted);
        if deleted < u64::from(PURGE_BATCH.get()) {
            break;
        }
    }
    assert_eq!(purged, [1, 1, 0]);
    assert_eq!(stored_tickets(&pool).await?, ["d"]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_deferred_poll_invalidates_a_late_dead_token() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone());
    let pending_receipts = PostgresPendingReceiptStore::new(pool.clone());
    let sender = FakePushSender::new();
    let alice = owner(&pool).await?;
    let names = ["fine", "gone", "reinstalled", "slow"];
    for name in names {
        registry.register(registration(alice, name, 0)).await?;
        sender.accept_without_receipt(token(name)).await;
    }

    let sent_at = at(1);
    let outcomes = sender
        .send(
            names
                .iter()
                .map(|name| PushMessage::new(token(name), "Reminder", "Body"))
                .collect(),
        )
        .await?;
    assert_eq!(
        pending_receipts.record_accepted(&outcomes, sent_at).await?,
        4
    );

    let dead = PushReceipt::Rejected(PushRejection::DeviceNotRegistered);
    sender
        .settle_receipt(token("fine"), PushReceipt::Delivered)
        .await;
    sender.settle_receipt(token("gone"), dead.clone()).await;
    sender.settle_receipt(token("reinstalled"), dead).await;
    registry
        .register(registration(alice, "reinstalled", 10))
        .await?;

    let due_at = sent_at + RECEIPT_POLL_DELAY;
    let poll =
        poll_pending_receipts(&sender, &pending_receipts, &registry, due_at, POLL_LIMIT).await?;
    assert_eq!(
        poll,
        ReceiptPoll {
            taken: 4,
            settled: 3,
            invalidated: 1
        }
    );
    let mut remaining = tokens(&registry, alice).await?;
    remaining.sort();
    assert_eq!(remaining, ["fine", "reinstalled", "slow"]);
    assert_eq!(stored_tickets(&pool).await?.len(), 1);

    let again =
        poll_pending_receipts(&sender, &pending_receipts, &registry, due_at, POLL_LIMIT).await?;
    assert_eq!(again.taken, 0, "the unsettled ticket waits a full delay");
    assert_eq!(
        pending_receipts
            .purge(due_at + RECEIPT_RETENTION, POLL_LIMIT)
            .await?,
        1
    );
    Ok(())
}
