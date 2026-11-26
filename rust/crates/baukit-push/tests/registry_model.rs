#![cfg(test)]
use std::{collections::BTreeSet, error::Error, num::NonZeroU32, path::PathBuf};

use baukit_push::{
    DeviceInstallSecret, DevicePlatform, DeviceRegistration, DeviceRegistry, DeviceToken,
    MemoryDeviceRegistry, PendingReceipt, PendingReceiptStore, PostgresDeviceRegistry,
    PostgresPendingReceiptStore, PushStoreError, PushTicketId, RegisteredDevice,
};
use baukit_test::PostgresTestContainer;
use jiff::{SignedDuration, Timestamp};
use proptest::{
    prelude::*,
    test_runner::{Config, TestCaseError, TestRunner},
};
use sqlx::PgPool;
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

const CAP: NonZeroU32 = NonZeroU32::new(2).expect("two is not zero");
const OWNERS: usize = 3;
const TOKENS: usize = 4;
const SECRETS: usize = 2;

fn at(minutes: i64) -> Timestamp {
    Timestamp::UNIX_EPOCH + SignedDuration::from_hours(500_000) + SignedDuration::from_mins(minutes)
}

fn token(index: usize) -> DeviceToken {
    DeviceToken::new(format!("device-token-{index}")).expect("valid test token")
}

fn secret(index: usize) -> DeviceInstallSecret {
    DeviceInstallSecret::new(format!("install-secret-{index:04}")).expect("valid test secret")
}

async fn fixture() -> Result<(PostgresTestContainer, PgPool, Vec<Uuid>), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(
        "CREATE TABLE owners (id UUID PRIMARY KEY);
         ALTER TABLE push_devices
             ADD CONSTRAINT push_devices_owner_fk
             FOREIGN KEY (owner_id) REFERENCES owners (id) ON DELETE CASCADE;",
    )
    .execute(&pool)
    .await?;
    let owners = (0..OWNERS)
        .map(|index| Uuid::from_u128(u128::try_from(index).unwrap_or_default() + 1))
        .collect::<Vec<_>>();
    for owner_id in &owners {
        sqlx::query("INSERT INTO owners (id) VALUES ($1)")
            .bind(owner_id)
            .execute(&pool)
            .await?;
    }
    Ok((fixture, pool, owners))
}

fn registration(owner_id: Uuid, token_index: usize, minutes: i64) -> DeviceRegistration {
    DeviceRegistration::new(
        owner_id,
        token(token_index),
        DevicePlatform::Android,
        at(minutes),
    )
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_token_moves_to_another_owner_only_with_its_install_secret() -> Result<(), TestError> {
    let (_container, pool, owners) = fixture().await?;
    let registry = PostgresDeviceRegistry::new(pool.clone());
    let (victim, attacker) = (owners[0], owners[1]);
    registry
        .register(registration(victim, 0, 0).with_install_secret(secret(0)))
        .await?;
    registry.register(registration(victim, 1, 0)).await?;
    for attempt in [
        registration(attacker, 0, 1),
        registration(attacker, 0, 1).with_install_secret(secret(1)),
    ] {
        assert_eq!(
            registry.register(attempt).await,
            Err(PushStoreError::TokenHeldByAnotherOwner)
        );
    }
    assert_eq!(
        registry
            .rotate(token(3), registration(attacker, 0, 1))
            .await,
        Err(PushStoreError::TokenHeldByAnotherOwner)
    );
    registry.erase_owner(attacker).await?;
    assert_eq!(registry.list_for_owner(victim).await?.len(), 2);

    registry
        .register(registration(attacker, 1, 2))
        .await
        .map_err(|error| format!("a legacy row without a secret still moves: {error}"))?;
    registry
        .register(registration(owners[2], 0, 3).with_install_secret(secret(0)))
        .await?;
    assert_eq!(registry.list_for_owner(owners[2]).await?.len(), 1);
    registry.erase_owner(attacker).await?;
    assert!(registry.list_for_owner(victim).await?.is_empty());
    assert_eq!(registry.list_for_owner(owners[2]).await?.len(), 1);
    Ok(())
}

#[derive(Clone, Debug)]
enum Action {
    Register {
        owner: usize,
        token: usize,
        secret: Option<usize>,
    },
    Rotate {
        owner: usize,
        previous: usize,
        token: usize,
        secret: Option<usize>,
    },
    Unregister {
        owner: usize,
        token: usize,
    },
    Invalidate {
        tokens: Vec<usize>,
        lag: i64,
    },
    EraseOwner {
        owner: usize,
    },
}

impl Action {
    fn registration(&self) -> Option<(usize, usize, Option<usize>)> {
        match self {
            Self::Register {
                owner,
                token,
                secret,
            }
            | Self::Rotate {
                owner,
                token,
                secret,
                ..
            } => Some((*owner, *token, *secret)),
            _ => None,
        }
    }
}

fn action() -> impl Strategy<Value = Action> {
    let owner = 0..OWNERS;
    let token = 0..TOKENS;
    let secret = prop::option::of(0..SECRETS);
    prop_oneof![
        3 => (owner.clone(), token.clone(), secret.clone())
            .prop_map(|(owner, token, secret)| Action::Register { owner, token, secret }),
        1 => (owner.clone(), token.clone(), token.clone(), secret).prop_map(
            |(owner, previous, token, secret)| Action::Rotate { owner, previous, token, secret }
        ),
        1 => (owner.clone(), token.clone()).prop_map(|(owner, token)| Action::Unregister { owner, token }),
        1 => (prop::collection::vec(token, 0..3), 0_i64..3)
            .prop_map(|(tokens, lag)| Action::Invalidate { tokens, lag }),
        1 => owner.prop_map(|owner| Action::EraseOwner { owner }),
    ]
}

struct Registries {
    postgres: PostgresDeviceRegistry,
    owners: Vec<Uuid>,
}

impl Registries {
    fn build(registration: DeviceRegistration, secret_index: Option<usize>) -> DeviceRegistration {
        match secret_index {
            Some(index) => registration.with_install_secret(secret(index)),
            None => registration,
        }
    }

    async fn snapshot(
        &self,
        registry: &dyn DeviceRegistry,
    ) -> Result<Vec<Vec<RegisteredDevice>>, TestError> {
        let mut devices = Vec::with_capacity(self.owners.len());
        for owner_id in &self.owners {
            devices.push(registry.list_for_owner(*owner_id).await?);
        }
        Ok(devices)
    }

    async fn apply(
        &self,
        registry: &dyn DeviceRegistry,
        action: &Action,
        step: i64,
    ) -> Result<String, TestError> {
        let result = match action {
            Action::Register {
                owner,
                token,
                secret,
            } => registry
                .register(Self::build(
                    registration(self.owners[*owner], *token, step),
                    *secret,
                ))
                .await
                .map(|outcome| format!("{outcome:?}")),
            Action::Rotate {
                owner,
                previous,
                token: next,
                secret,
            } => registry
                .rotate(
                    token(*previous),
                    Self::build(registration(self.owners[*owner], *next, step), *secret),
                )
                .await
                .map(|outcome| format!("{outcome:?}")),
            Action::Unregister {
                owner,
                token: index,
            } => registry
                .unregister(self.owners[*owner], token(*index))
                .await
                .map(|removed| removed.to_string()),
            Action::Invalidate { tokens, lag } => registry
                .invalidate(tokens.iter().copied().map(token).collect(), at(step - lag))
                .await
                .map(|removed| removed.to_string()),
            Action::EraseOwner { owner } => registry
                .erase_owner(self.owners[*owner])
                .await
                .map(|removed| removed.to_string()),
        };
        Ok(match result {
            Ok(value) => value,
            Err(PushStoreError::TokenHeldByAnotherOwner) => "refused".to_owned(),
            Err(error) => return Err(error.into()),
        })
    }

    fn check_invariants(devices: &[Vec<RegisteredDevice>]) -> Result<(), TestCaseError> {
        let mut seen = BTreeSet::new();
        for owned in devices {
            prop_assert!(owned.len() <= usize::try_from(CAP.get()).unwrap_or(usize::MAX));
            for device in owned {
                prop_assert!(
                    seen.insert(device.token.expose().to_owned()),
                    "a token has two owners"
                );
            }
        }
        Ok(())
    }

    fn holder(devices: &[Vec<RegisteredDevice>], token_index: usize) -> Option<usize> {
        let wanted = token(token_index);
        devices
            .iter()
            .position(|owned| owned.iter().any(|device| device.token == wanted))
    }
}

/// P-13: the PostgreSQL registry and the memory registry agree step by step, and the
/// shared invariants hold after every action.
#[test]
#[ignore = "requires Docker; mandatory in the full local gate"]
fn postgres_registry_matches_the_memory_registry() -> Result<(), TestError> {
    let runtime = tokio::runtime::Runtime::new()?;
    let (container, pool, owners) = runtime.block_on(fixture())?;
    let registries = Registries {
        postgres: PostgresDeviceRegistry::new(pool.clone()).with_devices_per_owner(CAP),
        owners,
    };
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..Config::with_cases(48)
    });
    runner.run(&prop::collection::vec(action(), 1..24), |actions| {
        runtime.block_on(async {
            sqlx::query("DELETE FROM push_devices")
                .execute(&pool)
                .await
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let memory = MemoryDeviceRegistry::new().with_devices_per_owner(CAP);
            let mut secrets: [Option<usize>; TOKENS] = [None; TOKENS];
            for (step, action) in actions.iter().enumerate() {
                let step = i64::try_from(step).unwrap_or(i64::MAX) * 10;
                let before = registries
                    .snapshot(&memory)
                    .await
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                let expected = registries
                    .apply(&memory, action, step)
                    .await
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                let actual = registries
                    .apply(&registries.postgres, action, step)
                    .await
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                prop_assert_eq!(&actual, &expected, "{:?}", action);
                if let Some((owner, token, secret)) = action.registration() {
                    let holder = Registries::holder(&before, token);
                    let foreign = holder.is_some_and(|holder| holder != owner);
                    let guarded = secrets[token].is_some_and(|stored| Some(stored) != secret);
                    if foreign && guarded {
                        prop_assert_eq!(&actual, "refused");
                    }
                    if actual != "refused" {
                        secrets[token] = match holder {
                            Some(holder) if holder == owner => secret.or(secrets[token]),
                            _ => secret,
                        };
                    }
                }
                let after = registries
                    .snapshot(&registries.postgres)
                    .await
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                Registries::check_invariants(&after)?;
                let mirrored = registries
                    .snapshot(&memory)
                    .await
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                prop_assert_eq!(after, mirrored);
            }
            Ok(())
        })
    })?;
    runtime.block_on(async move { drop(container) });
    Ok(())
}

fn pending(owner_id: Uuid, index: usize) -> PendingReceipt {
    PendingReceipt {
        ticket: PushTicketId::new(format!("ticket-{index:04}")).expect("valid ticket"),
        owner_id,
        token: token(index),
        sent_at: at(0),
    }
}

async fn drain(
    store: PostgresPendingReceiptStore,
    batch: NonZeroU32,
) -> Result<Vec<String>, PushStoreError> {
    let mut taken = Vec::new();
    loop {
        let receipts = store.take_due(at(1), at(30), batch).await?;
        if receipts.is_empty() {
            return Ok(taken);
        }
        taken.extend(
            receipts
                .iter()
                .map(|receipt| receipt.ticket.as_str().to_owned()),
        );
    }
}

/// P-17: concurrent pollers draining one window take every ticket exactly once.
#[test]
#[ignore = "requires Docker; mandatory in the full local gate"]
fn concurrent_pollers_take_every_ticket_once() -> Result<(), TestError> {
    let runtime = tokio::runtime::Runtime::new()?;
    let (container, pool, owners) = runtime.block_on(fixture())?;
    let store = PostgresPendingReceiptStore::new(pool.clone());
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..Config::with_cases(24)
    });
    runner.run(
        &(1_usize..40, 2_usize..8, 1_u32..5),
        |(tickets, pollers, batch)| {
            runtime.block_on(async {
                let fail = |error: &dyn std::fmt::Display| TestCaseError::fail(error.to_string());
                sqlx::query("DELETE FROM push_pending_receipts")
                    .execute(&pool)
                    .await
                    .map_err(|error| fail(&error))?;
                store
                    .record(
                        (0..tickets)
                            .map(|index| pending(owners[0], index))
                            .collect(),
                        at(0),
                    )
                    .await
                    .map_err(|error| fail(&error))?;
                let batch = NonZeroU32::new(batch).ok_or_else(|| fail(&"zero batch"))?;
                let mut tasks = tokio::task::JoinSet::new();
                for _ in 0..pollers {
                    tasks.spawn(drain(store.clone(), batch));
                }
                let mut taken = Vec::new();
                while let Some(result) = tasks.join_next().await {
                    taken.extend(
                        result
                            .map_err(|error| fail(&error))?
                            .map_err(|error| fail(&error))?,
                    );
                }
                let distinct = taken.iter().collect::<BTreeSet<_>>();
                prop_assert_eq!(distinct.len(), taken.len(), "a ticket was taken twice");
                prop_assert_eq!(taken.len(), tickets);
                Ok(())
            })
        },
    )?;
    runtime.block_on(async move { drop(container) });
    Ok(())
}
