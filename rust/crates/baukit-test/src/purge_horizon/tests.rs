use std::{collections::BTreeMap, sync::Mutex};

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    None,
    LowersHorizon,
    IgnoresLimit,
    SharedHorizon,
    RacyPull,
    KeepsHorizonOnErasure,
    ServesStaleCursor,
    SkipsPause,
}

#[derive(Debug)]
struct FakeError;

#[derive(Default)]
struct FakeOwner {
    counter: i64,
    rows: Vec<FakeRow>,
    horizon: Option<i64>,
}

struct FakeRow {
    revision: i64,
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct FakeState {
    next_owner: u64,
    owners: BTreeMap<u64, FakeOwner>,
    shared_horizon: Option<i64>,
}

struct FakeStore {
    fault: Fault,
    state: Mutex<FakeState>,
}

impl FakeStore {
    fn new(fault: Fault) -> Self {
        Self {
            fault,
            state: Mutex::new(FakeState::default()),
        }
    }

    fn with_owner<Value>(
        &self,
        owner: u64,
        action: impl FnOnce(&mut FakeOwner) -> Value,
    ) -> Result<Value, FakeError> {
        let mut state = self.state.lock().map_err(|_| FakeError)?;
        state.owners.get_mut(&owner).map(action).ok_or(FakeError)
    }

    fn horizon_of(&self, state: &FakeState, owner: u64) -> Option<i64> {
        if self.fault == Fault::SharedHorizon {
            return state.shared_horizon;
        }
        state.owners.get(&owner).and_then(|entry| entry.horizon)
    }

    fn check_cursor(&self, owner: u64, cursor: i64) -> Result<Option<i64>, FakeError> {
        let state = self.state.lock().map_err(|_| FakeError)?;
        let horizon = self.horizon_of(&state, owner).unwrap_or_default();
        let stale = cursor > 0 && cursor < horizon && self.fault != Fault::ServesStaleCursor;
        Ok(stale.then_some(horizon))
    }

    fn rows_above(&self, owner: u64, cursor: i64) -> Result<Vec<i64>, FakeError> {
        self.with_owner(owner, |entry| {
            entry
                .rows
                .iter()
                .map(|row| row.revision)
                .filter(|revision| *revision > cursor)
                .collect()
        })
    }

    fn raise(&self, state: &mut FakeState, owner: u64, revision: i64) {
        match self.fault {
            Fault::SharedHorizon => {
                state.shared_horizon = state.shared_horizon.max(Some(revision));
            }
            Fault::LowersHorizon => {
                if let Some(entry) = state.owners.get_mut(&owner) {
                    entry.horizon = Some(revision);
                }
            }
            _ => {
                if let Some(entry) = state.owners.get_mut(&owner) {
                    entry.horizon = entry.horizon.max(Some(revision));
                }
            }
        }
    }
}

impl PurgeHorizonAdapter for FakeStore {
    type Owner = u64;
    type Error = FakeError;

    async fn create_owner(&self) -> Result<u64, FakeError> {
        let mut state = self.state.lock().map_err(|_| FakeError)?;
        state.next_owner += 1;
        let owner = state.next_owner;
        state.owners.insert(owner, FakeOwner::default());
        Ok(owner)
    }

    async fn write_live_row(&self, owner: &u64) -> Result<i64, FakeError> {
        self.with_owner(*owner, |entry| {
            entry.counter += 1;
            entry.rows.push(FakeRow {
                revision: entry.counter,
                deleted_at: None,
            });
            entry.counter
        })
    }

    async fn write_tombstone(
        &self,
        owner: &u64,
        deleted_at: DateTime<Utc>,
    ) -> Result<i64, FakeError> {
        self.with_owner(*owner, |entry| {
            entry.counter += 2;
            entry.rows.push(FakeRow {
                revision: entry.counter,
                deleted_at: Some(deleted_at),
            });
            entry.counter
        })
    }

    async fn purge_batch(
        &self,
        cutoff: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> Result<u64, FakeError> {
        let mut state = self.state.lock().map_err(|_| FakeError)?;
        let mut expired = state
            .owners
            .iter()
            .flat_map(|(owner, entry)| {
                entry.rows.iter().filter_map(|row| {
                    row.deleted_at
                        .filter(|deleted_at| *deleted_at < cutoff)
                        .map(|deleted_at| (deleted_at, row.revision, *owner))
                })
            })
            .collect::<Vec<_>>();
        expired.sort_unstable();
        if self.fault != Fault::IgnoresLimit {
            expired.truncate(usize::try_from(limit.get()).map_err(|_| FakeError)?);
        }
        let mut batch_horizons = BTreeMap::<u64, i64>::new();
        for (_, revision, owner) in &expired {
            if let Some(entry) = state.owners.get_mut(owner) {
                entry.rows.retain(|row| row.revision != *revision);
            }
            let horizon = batch_horizons.entry(*owner).or_insert(*revision);
            *horizon = (*horizon).max(*revision);
        }
        for (owner, revision) in batch_horizons {
            self.raise(&mut state, owner, revision);
        }
        u64::try_from(expired.len()).map_err(|_| FakeError)
    }

    async fn pull(
        &self,
        owner: &u64,
        cursor: i64,
        pause: PullPause,
    ) -> Result<PurgeHorizonPull, FakeError> {
        if let Some(horizon_revision) = self.check_cursor(*owner, cursor)? {
            return Ok(PurgeHorizonPull::ResyncRequired { horizon_revision });
        }
        let snapshot = self.rows_above(*owner, cursor)?;
        match self.fault {
            Fault::SkipsPause => drop(pause),
            _ => pause.reached().await,
        }
        let revisions = if self.fault == Fault::RacyPull {
            self.rows_above(*owner, cursor)?
        } else {
            snapshot
        };
        Ok(PurgeHorizonPull::Page { revisions })
    }

    async fn purge_horizon(&self, owner: &u64) -> Result<Option<i64>, FakeError> {
        let state = self.state.lock().map_err(|_| FakeError)?;
        Ok(self.horizon_of(&state, *owner))
    }

    async fn erase_owner(&self, owner: &u64) -> Result<(), FakeError> {
        let mut state = self.state.lock().map_err(|_| FakeError)?;
        let removed = state.owners.remove(owner).ok_or(FakeError)?;
        if self.fault == Fault::KeepsHorizonOnErasure {
            state.owners.insert(
                *owner,
                FakeOwner {
                    horizon: removed.horizon,
                    ..FakeOwner::default()
                },
            );
        }
        Ok(())
    }
}

async fn violations(fault: Fault) -> Vec<String> {
    check_purge_horizon_conformance(&FakeStore::new(fault))
        .await
        .err()
        .map(|error| error.violations().to_vec())
        .unwrap_or_default()
}

async fn assert_detects(fault: Fault, expected: &str) {
    let found = violations(fault).await;
    assert!(
        found.iter().any(|violation| violation.contains(expected)),
        "{fault:?} should report {expected:?}, got {found:#?}"
    );
}

#[tokio::test]
async fn a_conforming_store_passes() {
    assert_eq!(violations(Fault::None).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_lowered_horizon_is_reported() {
    assert_detects(Fault::LowersHorizon, "after purging a lower revision").await;
}

#[tokio::test]
async fn an_unbounded_batch_is_reported() {
    assert_detects(
        Fault::IgnoresLimit,
        "batch bounds: a batch with limit 2 removed",
    )
    .await;
}

#[tokio::test]
async fn a_horizon_shared_between_owners_is_reported() {
    assert_detects(Fault::SharedHorizon, "rejected by another owner's horizon").await;
}

#[tokio::test]
async fn a_pull_that_reads_rows_after_a_purge_is_reported() {
    assert_detects(
        Fault::RacyPull,
        "omitted a tombstone purged while it was open",
    )
    .await;
}

#[tokio::test]
async fn a_horizon_left_after_erasure_is_reported() {
    assert_detects(
        Fault::KeepsHorizonOnErasure,
        "erasing an owner left its purge horizon",
    )
    .await;
}

#[tokio::test]
async fn a_stale_cursor_served_as_a_page_is_reported() {
    assert_detects(
        Fault::ServesStaleCursor,
        "a cursor below the horizon was served",
    )
    .await;
}

#[tokio::test]
async fn a_pull_without_the_pause_point_is_reported() {
    assert_detects(Fault::SkipsPause, "never called PullPause::reached").await;
}

#[test]
fn violations_do_not_leak_adapter_details() {
    let error = PurgeHorizonConformanceError {
        violations: vec!["erasure: erasing an owner failed".to_owned()],
    };
    assert_eq!(
        error.to_string(),
        "tombstone purge-horizon conformance failed:\n- erasure: erasing an owner failed\n"
    );
}
