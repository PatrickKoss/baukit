use std::collections::BTreeMap;

use serde_json::json;
use tokio::sync::Mutex;

use super::*;

const CREATED: u16 = 201;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    None,
    ReportsInProgress,
    SeparateTransactions,
    IgnoresFingerprint,
    RawFingerprint,
    ScopeWithoutOwner,
    ScopeWithoutOperation,
    RacyClaim,
    ReplaysExpired,
    IgnoresPurgeLimit,
    CleanupDeletesNothing,
    KeepsRecordsOnErasure,
    MemoryOnlySnapshots,
    SkipsCheckpoint,
}

#[derive(Debug)]
struct FakeError;

type ScopeKey = (Option<u64>, Option<u8>, String);

#[derive(Clone)]
struct FakeRecord {
    owner: u64,
    fingerprint: Value,
    snapshot: ReplaySnapshot,
    expired: bool,
}

#[derive(Default)]
struct FakeState {
    next_owner: u64,
    next_effect: u64,
    effects: BTreeMap<u64, u64>,
    records: BTreeMap<ScopeKey, FakeRecord>,
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

    fn scope(&self, request: &ReplayRequest<'_, u64>) -> ScopeKey {
        let owner = (self.fault != Fault::ScopeWithoutOwner).then_some(*request.owner);
        let operation =
            (self.fault != Fault::ScopeWithoutOperation).then_some(match request.operation {
                ReplayOperation::Primary => 0,
                ReplayOperation::Secondary => 1,
            });
        (owner, operation, request.key.to_owned())
    }

    fn fingerprint(&self, request: &ReplayRequest<'_, u64>) -> Result<Value, FakeError> {
        let input = if self.fault == Fault::RawFingerprint {
            Value::String(request.body.to_owned())
        } else {
            serde_json::from_str(request.body).map_err(|_| FakeError)?
        };
        Ok(json!({ "operation": format!("{:?}", request.operation), "input": input }))
    }

    fn lookup(&self, state: &FakeState, scope: &ScopeKey, fingerprint: &Value) -> Lookup {
        let Some(record) = state.records.get(scope) else {
            return Lookup::Missing;
        };
        if record.expired && self.fault != Fault::ReplaysExpired {
            return Lookup::Missing;
        }
        if record.fingerprint != *fingerprint && self.fault != Fault::IgnoresFingerprint {
            return Lookup::Found(ReplayOutcome::Conflict);
        }
        Lookup::Found(ReplayOutcome::Replayed(record.snapshot.clone()))
    }

    async fn checkpoint(&self, checkpoint: CommitCheckpoint) -> Result<(), FakeError> {
        if self.fault == Fault::SkipsCheckpoint {
            return Ok(());
        }
        checkpoint.reached().await.map_err(|_| FakeError)
    }
}

enum Lookup {
    Missing,
    Found(ReplayOutcome),
}

impl ReplaySafeMutationAdapter for FakeStore {
    type Owner = u64;
    type Error = FakeError;

    async fn create_owner(&self) -> Result<u64, FakeError> {
        let mut state = self.state.lock().await;
        state.next_owner += 1;
        let owner = state.next_owner;
        state.effects.insert(owner, 0);
        Ok(owner)
    }

    async fn execute(
        &self,
        request: ReplayRequest<'_, u64>,
        checkpoint: CommitCheckpoint,
    ) -> Result<ReplayOutcome, FakeError> {
        let scope = self.scope(&request);
        let fingerprint = self.fingerprint(&request)?;
        let mut state = if self.fault == Fault::ReportsInProgress {
            let Ok(state) = self.state.try_lock() else {
                return Ok(ReplayOutcome::InProgress);
            };
            state
        } else {
            self.state.lock().await
        };
        if let Lookup::Found(outcome) = self.lookup(&state, &scope, &fingerprint) {
            return Ok(outcome);
        }
        state.next_effect += 1;
        let snapshot = ReplaySnapshot {
            status: CREATED,
            body: json!({ "id": state.next_effect }),
        };
        if self.fault == Fault::SeparateTransactions {
            *state.effects.entry(*request.owner).or_default() += 1;
        }
        if self.fault == Fault::RacyClaim {
            drop(state);
            self.checkpoint(checkpoint).await?;
            state = self.state.lock().await;
        } else {
            self.checkpoint(checkpoint).await?;
        }
        if self.fault != Fault::SeparateTransactions {
            *state.effects.entry(*request.owner).or_default() += 1;
        }
        let record = FakeRecord {
            owner: *request.owner,
            fingerprint,
            snapshot: snapshot.clone(),
            expired: false,
        };
        state.records.insert(scope, record);
        Ok(ReplayOutcome::Applied(snapshot))
    }

    async fn effects(&self, owner: &u64) -> Result<u64, FakeError> {
        let state = self.state.lock().await;
        state.effects.get(owner).copied().ok_or(FakeError)
    }

    async fn replay_records(&self, owner: &u64) -> Result<u64, FakeError> {
        let state = self.state.lock().await;
        let count = state
            .records
            .values()
            .filter(|record| record.owner == *owner)
            .count();
        u64::try_from(count).map_err(|_| FakeError)
    }

    async fn expire_replay_records(&self, owner: &u64) -> Result<(), FakeError> {
        let mut state = self.state.lock().await;
        for record in state.records.values_mut() {
            record.expired |= record.owner == *owner;
        }
        Ok(())
    }

    async fn purge_expired(&self, limit: NonZeroU32) -> Result<u64, FakeError> {
        if self.fault == Fault::CleanupDeletesNothing {
            return Ok(0);
        }
        let mut state = self.state.lock().await;
        let mut expired = state
            .records
            .iter()
            .filter(|(_, record)| record.expired)
            .map(|(scope, _)| scope.clone())
            .collect::<Vec<_>>();
        if self.fault != Fault::IgnoresPurgeLimit {
            expired.truncate(usize::try_from(limit.get()).map_err(|_| FakeError)?);
        }
        for scope in &expired {
            state.records.remove(scope);
        }
        u64::try_from(expired.len()).map_err(|_| FakeError)
    }

    async fn erase_owner(&self, owner: &u64) -> Result<(), FakeError> {
        let mut state = self.state.lock().await;
        state.effects.remove(owner).ok_or(FakeError)?;
        if self.fault != Fault::KeepsRecordsOnErasure {
            state.records.retain(|_, record| record.owner != *owner);
        }
        Ok(())
    }

    async fn discard_transient_state(&self) -> Result<(), FakeError> {
        if self.fault == Fault::MemoryOnlySnapshots {
            self.state.lock().await.records.clear();
        }
        Ok(())
    }
}

fn inputs() -> ReplayConformanceInputs {
    ReplayConformanceInputs {
        input: r#"{"title":"first","amount":3}"#.to_owned(),
        equivalent_input: r#"{ "amount": 3, "title": "first" }"#.to_owned(),
        changed_input: r#"{"title":"first","amount":4}"#.to_owned(),
        secondary_input: r#"{"title":"first","amount":3}"#.to_owned(),
    }
}

async fn violations(fault: Fault) -> Vec<String> {
    check_replay_safe_mutation_conformance(&FakeStore::new(fault), &inputs())
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
async fn a_store_that_reports_in_progress_passes() {
    assert_eq!(
        violations(Fault::ReportsInProgress).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn an_effect_committed_outside_the_replay_transaction_is_reported() {
    assert_detects(
        Fault::SeparateTransactions,
        "rollback before commit: a rolled-back mutation left its effect",
    )
    .await;
}

#[tokio::test]
async fn a_replay_that_ignores_the_fingerprint_is_reported() {
    assert_detects(
        Fault::IgnoresFingerprint,
        "changed input: a changed body under the same key replayed the first result",
    )
    .await;
}

#[tokio::test]
async fn a_fingerprint_over_raw_bytes_is_reported() {
    assert_detects(
        Fault::RawFingerprint,
        "equivalent input: a retry of the same request reported a conflict",
    )
    .await;
}

#[tokio::test]
async fn a_scope_without_the_owner_is_reported() {
    assert_detects(
        Fault::ScopeWithoutOwner,
        "owner isolation: another owner's key blocked this owner's request",
    )
    .await;
}

#[tokio::test]
async fn a_scope_without_the_operation_is_reported() {
    assert_detects(
        Fault::ScopeWithoutOperation,
        "operation isolation: a key used by one operation stopped another operation",
    )
    .await;
}

#[tokio::test]
async fn a_claim_that_does_not_block_a_concurrent_request_is_reported() {
    assert_detects(
        Fault::RacyClaim,
        "simultaneous requests: both requests with one key applied an effect",
    )
    .await;
}

#[tokio::test]
async fn a_replayed_expired_record_is_reported() {
    assert_detects(
        Fault::ReplaysExpired,
        "expiry: an expired record still caused a conflict",
    )
    .await;
}

#[tokio::test]
async fn an_unbounded_cleanup_batch_is_reported() {
    assert_detects(
        Fault::IgnoresPurgeLimit,
        "bounded cleanup: a batch with limit 2 deleted 5 records; expected 2",
    )
    .await;
}

#[tokio::test]
async fn a_cleanup_that_deletes_nothing_is_reported() {
    assert_detects(
        Fault::CleanupDeletesNothing,
        "bounded cleanup: a batch with limit 2 deleted 0 records; expected 2",
    )
    .await;
}

#[tokio::test]
async fn records_left_after_erasure_are_reported() {
    assert_detects(
        Fault::KeepsRecordsOnErasure,
        "erasure: erasing an owner left its replay records",
    )
    .await;
}

#[tokio::test]
async fn snapshots_lost_on_restart_are_reported() {
    assert_detects(
        Fault::MemoryOnlySnapshots,
        "crash after commit: a retry applied the effect again instead of replaying",
    )
    .await;
}

#[tokio::test]
async fn a_mutation_without_the_checkpoint_is_reported() {
    let found = violations(Fault::SkipsCheckpoint).await;
    for expected in [
        "never called CommitCheckpoint::reached",
        "succeeded although CommitCheckpoint::reached failed",
    ] {
        assert!(
            found.iter().any(|violation| violation.contains(expected)),
            "expected {expected:?}, got {found:#?}"
        );
    }
}

#[tokio::test]
async fn a_paused_checkpoint_waits_for_release() {
    let (reached, reached_signal) = oneshot::channel();
    let (release_signal, release) = oneshot::channel();
    let checkpoint = CommitCheckpoint {
        mode: CheckpointMode::Pause(PauseGate { reached, release }),
    };
    let waiter = tokio::spawn(checkpoint.reached());
    reached_signal.await.expect("the checkpoint signals");
    release_signal.send(()).expect("the checkpoint waits");
    assert_eq!(waiter.await.expect("the task finishes"), Ok(()));
}

#[test]
fn violations_do_not_leak_adapter_details() {
    let error = ReplayConformanceError {
        violations: vec!["erasure: erasing an owner failed".to_owned()],
    };
    assert_eq!(
        error.to_string(),
        "replay-safe mutation conformance failed:\n- erasure: erasing an owner failed\n"
    );
}
