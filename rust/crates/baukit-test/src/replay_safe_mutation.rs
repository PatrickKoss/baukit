use std::{error::Error as StdError, fmt, num::NonZeroU32, pin::pin, time::Duration};

use serde_json::Value;
use tokio::sync::oneshot;
use uuid::Uuid;

const CLEANUP_LIMIT: NonZeroU32 = NonZeroU32::new(2).expect("two is not zero");
const CLEANUP_RECORDS: u64 = 5;
const DRAIN_LIMIT: NonZeroU32 = NonZeroU32::new(100).expect("one hundred is not zero");
const MAX_DRAIN_BATCHES: usize = 1_000;
const CONCURRENT_OVERLAP: Duration = Duration::from_millis(250);
const RACE_DEADLINE: Duration = Duration::from_secs(30);

/// Which of the adapter's two operations a request runs.
///
/// Map each to one product operation, for example `Primary` to a create route
/// and `Secondary` to another write for the same owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayOperation {
    /// The operation that receives [`ReplayConformanceInputs::input`].
    Primary,
    /// The operation that receives [`ReplayConformanceInputs::secondary_input`].
    Secondary,
}

/// One keyed mutation request passed to [`ReplaySafeMutationAdapter::execute`].
#[derive(Debug)]
pub struct ReplayRequest<'a, Owner> {
    /// The caller scope that owns the replay record.
    pub owner: &'a Owner,
    /// The operation the request runs.
    pub operation: ReplayOperation,
    /// The `Idempotency-Key` value. The check uses UUID v4 strings.
    pub key: &'a str,
    /// The request body as JSON text, parsed and normalized by the product.
    pub body: &'a str,
}

impl<Owner> Clone for ReplayRequest<'_, Owner> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Owner> Copy for ReplayRequest<'_, Owner> {}

/// The durable result a product returns for the first execution and every replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaySnapshot {
    /// HTTP status of the original response.
    pub status: u16,
    /// Response body of the original response.
    pub body: Value,
}

/// What one keyed request did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayOutcome {
    /// This request applied the effect and stored its snapshot.
    Applied(ReplaySnapshot),
    /// This request returned the snapshot stored by an earlier request.
    Replayed(ReplaySnapshot),
    /// The key was used in this scope with a different request fingerprint.
    Conflict,
    /// Another request with the same key has not finished yet.
    InProgress,
}

/// Point a product mutation must reach after it has written its effect and
/// replay record and before it commits.
///
/// Outside the concurrency and rollback cases it returns immediately.
#[derive(Debug, Default)]
pub struct CommitCheckpoint {
    mode: CheckpointMode,
}

#[derive(Debug, Default)]
enum CheckpointMode {
    #[default]
    Pass,
    Pause(PauseGate),
    Rollback,
}

#[derive(Debug)]
struct PauseGate {
    reached: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl CommitCheckpoint {
    /// Creates a checkpoint that returns immediately.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            mode: CheckpointMode::Pass,
        }
    }

    /// Signals that the mutation is ready to commit and waits for the harness.
    ///
    /// Call it inside the mutation's transaction, after the effect and the
    /// replay record are written and before `COMMIT`.
    ///
    /// # Errors
    ///
    /// Returns [`InjectedRollback`] when the harness asks the mutation to fail.
    /// The adapter must roll the transaction back and return an error.
    pub async fn reached(self) -> Result<(), InjectedRollback> {
        match self.mode {
            CheckpointMode::Pass => Ok(()),
            CheckpointMode::Rollback => Err(InjectedRollback),
            CheckpointMode::Pause(gate) => {
                if gate.reached.send(()).is_ok() {
                    let _ = gate.release.await;
                }
                Ok(())
            }
        }
    }
}

/// The harness asked the mutation to fail before commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InjectedRollback;

impl fmt::Display for InjectedRollback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("injected failure before commit")
    }
}

impl StdError for InjectedRollback {}

/// Request bodies the check sends. Each is JSON text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayConformanceInputs {
    /// A valid body for [`ReplayOperation::Primary`].
    pub input: String,
    /// The same request as `input` in another encoding, such as reordered
    /// members and different whitespace. It must replay, not conflict.
    pub equivalent_input: String,
    /// A valid body for [`ReplayOperation::Primary`] that differs from `input`
    /// in meaning. It must conflict under the same key.
    pub changed_input: String,
    /// A valid body for [`ReplayOperation::Secondary`].
    pub secondary_input: String,
}

/// Product adapter exercised by the replay-safe mutation conformance check.
///
/// Bind the adapter to a database that holds no replay records another test
/// needs, because the cleanup case purges every expired record.
#[allow(async_fn_in_trait)]
pub trait ReplaySafeMutationAdapter {
    /// Product owner key, the caller scope.
    type Owner: Clone;
    /// Product failure. Its message never appears in harness output.
    type Error;

    /// Creates an owner with no effects and no replay records.
    async fn create_owner(&self) -> Result<Self::Owner, Self::Error>;

    /// Runs one keyed mutation through the product's replay path in one
    /// transaction, calling `checkpoint.reached()` just before commit.
    async fn execute(
        &self,
        request: ReplayRequest<'_, Self::Owner>,
        checkpoint: CommitCheckpoint,
    ) -> Result<ReplayOutcome, Self::Error>;

    /// Counts the committed effects of both operations for the owner.
    async fn effects(&self, owner: &Self::Owner) -> Result<u64, Self::Error>;

    /// Counts the owner's stored replay records, expired ones included.
    async fn replay_records(&self, owner: &Self::Owner) -> Result<u64, Self::Error>;

    /// Moves every replay record of the owner past its retention horizon.
    async fn expire_replay_records(&self, owner: &Self::Owner) -> Result<(), Self::Error>;

    /// Runs one cleanup batch that deletes at most `limit` expired replay
    /// records across all owners, and returns how many it deleted.
    async fn purge_expired(&self, limit: NonZeroU32) -> Result<u64, Self::Error>;

    /// Runs the product's owner erasure.
    async fn erase_owner(&self, owner: &Self::Owner) -> Result<(), Self::Error>;

    /// Drops process-local state, such as caches or pools, as a restart would,
    /// without deleting stored rows.
    async fn discard_transient_state(&self) -> Result<(), Self::Error>;
}

/// Violations found by the replay-safe mutation conformance check.
///
/// Messages never contain adapter errors, owner keys, idempotency keys, or
/// request and response bodies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayConformanceError {
    violations: Vec<String>,
}

impl ReplayConformanceError {
    /// Returns violations in check order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for ReplayConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "replay-safe mutation conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl StdError for ReplayConformanceError {}

/// Runs every replay-safe mutation case against the adapter.
///
/// The cases cover a lost response, an equivalent body, changed input under
/// the same key, simultaneous same-key requests, owner and operation
/// isolation, a rollback before commit, a crash after commit, expiry, bounded
/// cleanup, and owner erasure.
///
/// # Errors
///
/// Returns every violation found. A failing adapter call ends its case.
pub async fn check_replay_safe_mutation_conformance<Adapter>(
    adapter: &Adapter,
    inputs: &ReplayConformanceInputs,
) -> Result<(), ReplayConformanceError>
where
    Adapter: ReplaySafeMutationAdapter,
{
    let harness = Harness { adapter, inputs };
    let mut violations = Vec::new();
    let cases = [
        harness.lost_response(&mut violations).await,
        harness.equivalent_input(&mut violations).await,
        harness.changed_input(&mut violations).await,
        harness.simultaneous_requests(&mut violations).await,
        harness.owner_isolation(&mut violations).await,
        harness.operation_isolation(&mut violations).await,
        harness.rollback_before_commit(&mut violations).await,
        harness.crash_after_commit(&mut violations).await,
        harness.expiry(&mut violations).await,
        harness.bounded_cleanup(&mut violations).await,
        harness.erasure(&mut violations).await,
    ];
    violations.extend(cases.into_iter().filter_map(Result::err));
    if violations.is_empty() {
        Ok(())
    } else {
        Err(ReplayConformanceError { violations })
    }
}

/// Panics when the adapter fails the replay-safe mutation conformance check.
///
/// # Panics
///
/// Panics with every conformance violation.
pub async fn assert_replay_safe_mutation_conformance<Adapter>(
    adapter: &Adapter,
    inputs: &ReplayConformanceInputs,
) where
    Adapter: ReplaySafeMutationAdapter,
{
    if let Err(error) = check_replay_safe_mutation_conformance(adapter, inputs).await {
        panic!("{error}");
    }
}

type CaseResult = Result<(), String>;

struct Harness<'a, Adapter> {
    adapter: &'a Adapter,
    inputs: &'a ReplayConformanceInputs,
}

impl<Adapter> Harness<'_, Adapter>
where
    Adapter: ReplaySafeMutationAdapter,
{
    async fn lost_response(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "lost response";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&owner, &key, CASE).await?;
        self.expect_replay(
            &owner,
            &key,
            &self.inputs.input,
            &original,
            CASE,
            violations,
        )
        .await?;
        self.expect_counts(&owner, 1, 1, CASE, violations).await
    }

    async fn equivalent_input(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "equivalent input";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&owner, &key, CASE).await?;
        let body = &self.inputs.equivalent_input;
        self.expect_replay(&owner, &key, body, &original, CASE, violations)
            .await?;
        self.expect_counts(&owner, 1, 1, CASE, violations).await
    }

    async fn changed_input(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "changed input";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&owner, &key, CASE).await?;
        let changed = &self.inputs.changed_input;
        match self.primary(&owner, &key, changed, CASE).await? {
            ReplayOutcome::Conflict => {}
            ReplayOutcome::Replayed(_) => violations.push(format!(
                "{CASE}: a changed body under the same key replayed the first result"
            )),
            ReplayOutcome::Applied(_) => violations.push(format!(
                "{CASE}: a changed body under the same key applied a second effect"
            )),
            ReplayOutcome::InProgress => violations.push(format!(
                "{CASE}: a changed body under a finished key reported in progress"
            )),
        }
        self.expect_replay(
            &owner,
            &key,
            &self.inputs.input,
            &original,
            CASE,
            violations,
        )
        .await?;
        self.expect_counts(&owner, 1, 1, CASE, violations).await
    }

    async fn simultaneous_requests(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "simultaneous requests";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let request = self.request(&owner, ReplayOperation::Primary, &key, &self.inputs.input);
        let race = race_same_key(self.adapter, request);
        let Ok(outcome) = tokio::time::timeout(RACE_DEADLINE, race).await else {
            return Err(format!(
                "{CASE}: two requests with one key did not finish within {RACE_DEADLINE:?}"
            ));
        };
        if !outcome.checkpoint_reached {
            return Err(format!(
                "{CASE}: the mutation never called CommitCheckpoint::reached before commit"
            ));
        }
        let first = step(outcome.first, CASE, "running the first request")?;
        let ReplayOutcome::Applied(original) = first else {
            return Err(format!(
                "{CASE}: the first request did not apply its effect"
            ));
        };
        match step(outcome.second, CASE, "running the second request")? {
            ReplayOutcome::Replayed(snapshot) if snapshot == original => {}
            ReplayOutcome::InProgress => {}
            ReplayOutcome::Replayed(_) => violations.push(format!(
                "{CASE}: the waiting request replayed a different result"
            )),
            ReplayOutcome::Applied(_) => violations.push(format!(
                "{CASE}: both requests with one key applied an effect"
            )),
            ReplayOutcome::Conflict => violations.push(format!(
                "{CASE}: an identical request with the same key reported a conflict"
            )),
        }
        self.expect_replay(
            &owner,
            &key,
            &self.inputs.input,
            &original,
            CASE,
            violations,
        )
        .await?;
        self.expect_counts(&owner, 1, 1, CASE, violations).await
    }

    async fn owner_isolation(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "owner isolation";
        let first = self.owner(CASE).await?;
        let second = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&first, &key, CASE).await?;
        let changed = &self.inputs.changed_input;
        match self.primary(&second, &key, changed, CASE).await? {
            ReplayOutcome::Applied(_) => {}
            ReplayOutcome::Replayed(snapshot) if snapshot == original => violations.push(format!(
                "{CASE}: another owner received the first owner's stored result"
            )),
            ReplayOutcome::Conflict => violations.push(format!(
                "{CASE}: another owner's key blocked this owner's request"
            )),
            ReplayOutcome::Replayed(_) | ReplayOutcome::InProgress => violations.push(format!(
                "{CASE}: a key first used by this owner did not apply its effect"
            )),
        }
        self.expect_counts(&second, 1, 1, CASE, violations).await?;
        self.expect_replay(
            &first,
            &key,
            &self.inputs.input,
            &original,
            CASE,
            violations,
        )
        .await?;
        self.expect_counts(&first, 1, 1, CASE, violations).await
    }

    async fn operation_isolation(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "operation isolation";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&owner, &key, CASE).await?;
        let secondary = self.request(
            &owner,
            ReplayOperation::Secondary,
            &key,
            &self.inputs.secondary_input,
        );
        let outcome = step(
            self.adapter
                .execute(secondary, CommitCheckpoint::none())
                .await,
            CASE,
            "running the secondary operation",
        )?;
        if !matches!(outcome, ReplayOutcome::Applied(_)) {
            violations.push(format!(
                "{CASE}: a key used by one operation stopped another operation from applying"
            ));
        }
        self.expect_replay(
            &owner,
            &key,
            &self.inputs.input,
            &original,
            CASE,
            violations,
        )
        .await?;
        self.expect_counts(&owner, 2, 2, CASE, violations).await
    }

    async fn rollback_before_commit(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "rollback before commit";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let request = self.request(&owner, ReplayOperation::Primary, &key, &self.inputs.input);
        let rollback = CommitCheckpoint {
            mode: CheckpointMode::Rollback,
        };
        if self.adapter.execute(request, rollback).await.is_ok() {
            return Err(format!(
                "{CASE}: the mutation succeeded although CommitCheckpoint::reached failed"
            ));
        }
        let effects = self.effects(&owner, CASE).await?;
        let records = self.records(&owner, CASE).await?;
        if effects != 0 {
            violations.push(format!("{CASE}: a rolled-back mutation left its effect"));
        }
        if records != 0 {
            violations.push(format!(
                "{CASE}: a rolled-back mutation left a replay record"
            ));
        }
        if effects != 0 || records != 0 {
            return Ok(());
        }
        self.apply_primary(&owner, &key, CASE).await?;
        self.expect_counts(&owner, 1, 1, CASE, violations).await
    }

    async fn crash_after_commit(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "crash after commit";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&owner, &key, CASE).await?;
        step(
            self.adapter.discard_transient_state().await,
            CASE,
            "discarding process-local state",
        )?;
        self.expect_replay(
            &owner,
            &key,
            &self.inputs.input,
            &original,
            CASE,
            violations,
        )
        .await?;
        self.expect_counts(&owner, 1, 1, CASE, violations).await
    }

    async fn expiry(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "expiry";
        let owner = self.owner(CASE).await?;
        let key = new_key();
        let original = self.apply_primary(&owner, &key, CASE).await?;
        step(
            self.adapter.expire_replay_records(&owner).await,
            CASE,
            "expiring replay records",
        )?;
        let changed = &self.inputs.changed_input;
        let renewed = match self.primary(&owner, &key, changed, CASE).await? {
            ReplayOutcome::Applied(snapshot) => snapshot,
            ReplayOutcome::Conflict => {
                violations.push(format!("{CASE}: an expired record still caused a conflict"));
                return Ok(());
            }
            ReplayOutcome::Replayed(snapshot) if snapshot == original => {
                violations.push(format!("{CASE}: an expired record was replayed"));
                return Ok(());
            }
            ReplayOutcome::Replayed(_) | ReplayOutcome::InProgress => {
                violations.push(format!(
                    "{CASE}: a request after expiry did not run as a new request"
                ));
                return Ok(());
            }
        };
        self.expect_counts(&owner, 2, 1, CASE, violations).await?;
        self.expect_replay(&owner, &key, changed, &renewed, CASE, violations)
            .await
    }

    async fn bounded_cleanup(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "bounded cleanup";
        self.drain(CASE).await?;
        let expired = self.owner(CASE).await?;
        let live = self.owner(CASE).await?;
        for _ in 0..CLEANUP_RECORDS {
            self.apply_primary(&expired, &new_key(), CASE).await?;
        }
        let live_key = new_key();
        let live_snapshot = self.apply_primary(&live, &live_key, CASE).await?;
        step(
            self.adapter.expire_replay_records(&expired).await,
            CASE,
            "expiring replay records",
        )?;

        let limit = u64::from(CLEANUP_LIMIT.get());
        let mut remaining = CLEANUP_RECORDS;
        while remaining > 0 {
            let deleted = step(
                self.adapter.purge_expired(CLEANUP_LIMIT).await,
                CASE,
                "purging expired records",
            )?;
            let expected = remaining.min(limit);
            if deleted != expected {
                violations.push(format!(
                    "{CASE}: a batch with limit {limit} deleted {deleted} records; expected {expected}"
                ));
                break;
            }
            remaining -= deleted;
            if self.records(&expired, CASE).await? != remaining {
                violations.push(format!(
                    "{CASE}: the reported count did not match the records removed"
                ));
                break;
            }
        }
        if self.records(&expired, CASE).await? != 0 {
            violations.push(format!("{CASE}: cleanup left expired records"));
        }
        if self.records(&live, CASE).await? != 1 {
            violations.push(format!(
                "{CASE}: cleanup removed a record before it expired"
            ));
        }
        let original = &self.inputs.input;
        self.expect_replay(&live, &live_key, original, &live_snapshot, CASE, violations)
            .await
    }

    async fn erasure(&self, violations: &mut Vec<String>) -> CaseResult {
        const CASE: &str = "erasure";
        let erased = self.owner(CASE).await?;
        let kept = self.owner(CASE).await?;
        let key = new_key();
        self.apply_primary(&erased, &key, CASE).await?;
        let kept_snapshot = self.apply_primary(&kept, &key, CASE).await?;
        step(
            self.adapter.erase_owner(&erased).await,
            CASE,
            "erasing an owner",
        )?;
        if self.records(&erased, CASE).await? != 0 {
            violations.push(format!("{CASE}: erasing an owner left its replay records"));
        }
        self.expect_counts(&kept, 1, 1, CASE, violations).await?;
        let original = &self.inputs.input;
        self.expect_replay(&kept, &key, original, &kept_snapshot, CASE, violations)
            .await
    }

    async fn owner(&self, case: &str) -> Result<Adapter::Owner, String> {
        step(self.adapter.create_owner().await, case, "creating an owner")
    }

    fn request<'r>(
        &self,
        owner: &'r Adapter::Owner,
        operation: ReplayOperation,
        key: &'r str,
        body: &'r str,
    ) -> ReplayRequest<'r, Adapter::Owner> {
        ReplayRequest {
            owner,
            operation,
            key,
            body,
        }
    }

    async fn primary(
        &self,
        owner: &Adapter::Owner,
        key: &str,
        body: &str,
        case: &str,
    ) -> Result<ReplayOutcome, String> {
        let request = self.request(owner, ReplayOperation::Primary, key, body);
        step(
            self.adapter
                .execute(request, CommitCheckpoint::none())
                .await,
            case,
            "running a request",
        )
    }

    async fn apply_primary(
        &self,
        owner: &Adapter::Owner,
        key: &str,
        case: &str,
    ) -> Result<ReplaySnapshot, String> {
        match self.primary(owner, key, &self.inputs.input, case).await? {
            ReplayOutcome::Applied(snapshot) => Ok(snapshot),
            _ => Err(format!("{case}: a request with a new key did not apply")),
        }
    }

    async fn expect_replay(
        &self,
        owner: &Adapter::Owner,
        key: &str,
        body: &str,
        original: &ReplaySnapshot,
        case: &str,
        violations: &mut Vec<String>,
    ) -> CaseResult {
        match self.primary(owner, key, body, case).await? {
            ReplayOutcome::Replayed(snapshot) if &snapshot == original => {}
            ReplayOutcome::Replayed(_) => violations.push(format!(
                "{case}: a retry replayed a result that differs from the original"
            )),
            ReplayOutcome::Applied(_) => violations.push(format!(
                "{case}: a retry applied the effect again instead of replaying"
            )),
            ReplayOutcome::Conflict => violations.push(format!(
                "{case}: a retry of the same request reported a conflict"
            )),
            ReplayOutcome::InProgress => violations.push(format!(
                "{case}: a retry after the original finished reported in progress"
            )),
        }
        Ok(())
    }

    async fn expect_counts(
        &self,
        owner: &Adapter::Owner,
        effects: u64,
        records: u64,
        case: &str,
        violations: &mut Vec<String>,
    ) -> CaseResult {
        let actual_effects = self.effects(owner, case).await?;
        if actual_effects != effects {
            violations.push(format!(
                "{case}: the owner has {actual_effects} effects; expected {effects}"
            ));
        }
        let actual_records = self.records(owner, case).await?;
        if actual_records != records {
            violations.push(format!(
                "{case}: the owner has {actual_records} replay records; expected {records}"
            ));
        }
        Ok(())
    }

    async fn effects(&self, owner: &Adapter::Owner, case: &str) -> Result<u64, String> {
        step(self.adapter.effects(owner).await, case, "counting effects")
    }

    async fn records(&self, owner: &Adapter::Owner, case: &str) -> Result<u64, String> {
        step(
            self.adapter.replay_records(owner).await,
            case,
            "counting replay records",
        )
    }

    async fn drain(&self, case: &str) -> CaseResult {
        let limit = u64::from(DRAIN_LIMIT.get());
        for _ in 0..MAX_DRAIN_BATCHES {
            let deleted = step(
                self.adapter.purge_expired(DRAIN_LIMIT).await,
                case,
                "purging expired records",
            )?;
            if deleted > limit {
                return Err(format!(
                    "{case}: a batch with limit {limit} deleted {deleted} records"
                ));
            }
            if deleted == 0 {
                return Ok(());
            }
        }
        Err(format!(
            "{case}: cleanup did not finish within {MAX_DRAIN_BATCHES} batches"
        ))
    }
}

struct RaceOutcome<Error> {
    checkpoint_reached: bool,
    first: Result<ReplayOutcome, Error>,
    second: Result<ReplayOutcome, Error>,
}

async fn race_same_key<Adapter>(
    adapter: &Adapter,
    request: ReplayRequest<'_, Adapter::Owner>,
) -> RaceOutcome<Adapter::Error>
where
    Adapter: ReplaySafeMutationAdapter,
{
    let (reached, reached_signal) = oneshot::channel();
    let (release_signal, release) = oneshot::channel();
    let paused = CommitCheckpoint {
        mode: CheckpointMode::Pause(PauseGate { reached, release }),
    };
    let first = adapter.execute(request, paused);
    let second = async {
        let checkpoint_reached = reached_signal.await.is_ok();
        let mut second = pin!(adapter.execute(request, CommitCheckpoint::none()));
        let early = tokio::time::timeout(CONCURRENT_OVERLAP, &mut second).await;
        let _ = release_signal.send(());
        let second = match early {
            Ok(result) => result,
            Err(_) => second.await,
        };
        (checkpoint_reached, second)
    };
    let (first, (checkpoint_reached, second)) = tokio::join!(first, second);
    RaceOutcome {
        checkpoint_reached,
        first,
        second,
    }
}

fn new_key() -> String {
    Uuid::new_v4().to_string()
}

fn step<Output, Error>(
    result: Result<Output, Error>,
    case: &str,
    action: &str,
) -> Result<Output, String> {
    result.map_err(|_| format!("{case}: {action} failed"))
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "sqlx-postgres"))]
mod postgres_tests;
