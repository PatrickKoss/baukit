use std::{fmt, num::NonZeroU32, pin::pin, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::oneshot;

const BATCH_LIMIT: NonZeroU32 = NonZeroU32::new(2).expect("two is not zero");
const BATCH_TOMBSTONES: usize = 5;
const DRAIN_LIMIT: NonZeroU32 = NonZeroU32::new(100).expect("one hundred is not zero");
const MAX_DRAIN_BATCHES: usize = 1_000;
const EXPIRED_AGE: TimeDelta = TimeDelta::days(2);
const RETENTION: TimeDelta = TimeDelta::days(1);
const FAR_FUTURE: TimeDelta = TimeDelta::days(1);
const PURGE_OVERLAP: Duration = Duration::from_millis(250);
const RACE_DEADLINE: Duration = Duration::from_secs(30);
const BUSY_OWNER_LIVE_ROWS: usize = 3;

/// Result of one product pull as the purge-horizon conformance check sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PurgeHorizonPull {
    /// The pull was served. `revisions` lists every returned row's revision,
    /// live rows and tombstones alike.
    Page {
        /// Revisions of the returned rows.
        revisions: Vec<i64>,
    },
    /// The product rejected the cursor with `resync_required`.
    ResyncRequired {
        /// The horizon carried in `details.horizonRevision`.
        horizon_revision: i64,
    },
}

/// Pause point a product pull must reach between its cursor check and its row reads.
///
/// Outside the concurrency case it is a no-op.
#[derive(Debug, Default)]
pub struct PullPause {
    gate: Option<PauseGate>,
}

#[derive(Debug)]
struct PauseGate {
    reached: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl PullPause {
    /// Creates a pause that returns immediately.
    #[must_use]
    pub const fn none() -> Self {
        Self { gate: None }
    }

    /// Signals that the cursor check is done and waits for the harness.
    ///
    /// Call it inside the pull's database transaction, after the horizon check
    /// and before reading any row.
    pub async fn reached(self) {
        let Some(gate) = self.gate else {
            return;
        };
        if gate.reached.send(()).is_ok() {
            let _ = gate.release.await;
        }
    }
}

/// Product adapter exercised by the tombstone purge-horizon conformance check.
///
/// Bind the adapter to a database with no other tombstones. The check purges
/// every table the adapter covers, so it must not share the database with
/// data another test needs.
#[allow(async_fn_in_trait)]
pub trait PurgeHorizonAdapter {
    /// Product owner key.
    type Owner: Clone;
    /// Product failure. Its message never appears in harness output.
    type Error;

    /// Creates an owner with a revision counter and no rows.
    async fn create_owner(&self) -> Result<Self::Owner, Self::Error>;

    /// Writes one live syncable row and returns its revision.
    async fn write_live_row(&self, owner: &Self::Owner) -> Result<i64, Self::Error>;

    /// Writes one row, tombstones it with `deleted_at`, and returns the
    /// tombstone's revision.
    async fn write_tombstone(
        &self,
        owner: &Self::Owner,
        deleted_at: DateTime<Utc>,
    ) -> Result<i64, Self::Error>;

    /// Runs one purge transaction that removes at most `limit` tombstones
    /// deleted before `cutoff`, and returns how many it removed.
    async fn purge_batch(
        &self,
        cutoff: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> Result<u64, Self::Error>;

    /// Runs the product pull for `cursor`, calling `pause.reached()` after the
    /// horizon check and before reading rows.
    async fn pull(
        &self,
        owner: &Self::Owner,
        cursor: i64,
        pause: PullPause,
    ) -> Result<PurgeHorizonPull, Self::Error>;

    /// Reads the owner's stored purge horizon. `None` means no horizon row.
    async fn purge_horizon(&self, owner: &Self::Owner) -> Result<Option<i64>, Self::Error>;

    /// Runs the product's owner erasure.
    async fn erase_owner(&self, owner: &Self::Owner) -> Result<(), Self::Error>;
}

/// Violations found by the purge-horizon conformance check.
///
/// Messages never contain adapter errors, owner keys, or row data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeHorizonConformanceError {
    violations: Vec<String>,
}

impl PurgeHorizonConformanceError {
    /// Returns violations in check order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for PurgeHorizonConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "tombstone purge-horizon conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PurgeHorizonConformanceError {}

/// Runs every purge-horizon case against the adapter.
///
/// The cases cover horizon monotonicity, cursors at, above, and below the
/// horizon, bounded purge batches, owner isolation, a purge that races an open
/// pull, and owner erasure.
///
/// # Errors
///
/// Returns every violation found. A failing adapter call ends its case.
pub async fn check_purge_horizon_conformance<Adapter>(
    adapter: &Adapter,
) -> Result<(), PurgeHorizonConformanceError>
where
    Adapter: PurgeHorizonAdapter,
{
    let clock = Clock::new(Utc::now());
    let mut violations = Vec::new();
    let cases = [
        monotonic_horizon_and_cursor_boundaries(adapter, clock, &mut violations).await,
        bounded_batches(adapter, clock, &mut violations).await,
        owner_isolation(adapter, clock, &mut violations).await,
        purge_racing_an_open_pull(adapter, clock, &mut violations).await,
        erasure(adapter, clock, &mut violations).await,
    ];
    violations.extend(cases.into_iter().filter_map(Result::err));
    if violations.is_empty() {
        Ok(())
    } else {
        Err(PurgeHorizonConformanceError { violations })
    }
}

/// Panics when the adapter fails the purge-horizon conformance check.
///
/// # Panics
///
/// Panics with every conformance violation.
pub async fn assert_purge_horizon_conformance<Adapter>(adapter: &Adapter)
where
    Adapter: PurgeHorizonAdapter,
{
    if let Err(error) = check_purge_horizon_conformance(adapter).await {
        panic!("{error}");
    }
}

#[derive(Clone, Copy)]
struct Clock {
    now: DateTime<Utc>,
}

impl Clock {
    const fn new(now: DateTime<Utc>) -> Self {
        Self { now }
    }

    fn expired(self) -> DateTime<Utc> {
        self.now - EXPIRED_AGE
    }

    fn cutoff(self) -> DateTime<Utc> {
        self.now - RETENTION
    }

    fn everything(self) -> DateTime<Utc> {
        self.now + FAR_FUTURE
    }
}

type CaseResult = Result<(), String>;

async fn monotonic_horizon_and_cursor_boundaries<Adapter>(
    adapter: &Adapter,
    clock: Clock,
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    const CASE: &str = "horizon monotonicity";
    drain(adapter, clock.everything(), CASE).await?;
    let owner = step(adapter.create_owner().await, CASE, "creating an owner")?;
    let live = step(
        adapter.write_live_row(&owner).await,
        CASE,
        "writing a live row",
    )?;
    let recent = step(
        adapter.write_tombstone(&owner, clock.now).await,
        CASE,
        "writing a recent tombstone",
    )?;
    let expired = step(
        adapter.write_tombstone(&owner, clock.expired()).await,
        CASE,
        "writing an expired tombstone",
    )?;
    if !(live < recent && recent < expired) {
        return Err(format!(
            "{CASE}: revisions of consecutive writes did not increase"
        ));
    }
    if horizon(adapter, &owner, CASE).await?.is_some() {
        violations.push(format!("{CASE}: an owner had a horizon before any purge"));
    }

    drain(adapter, clock.cutoff(), CASE).await?;
    expect_horizon(
        adapter,
        &owner,
        expired,
        CASE,
        "after the first purge",
        violations,
    )
    .await?;
    let later = step(
        adapter.write_live_row(&owner).await,
        CASE,
        "writing a live row",
    )?;
    cursor_boundaries(adapter, &owner, expired, [live, later], violations).await?;

    drain(adapter, clock.everything(), CASE).await?;
    expect_horizon(
        adapter,
        &owner,
        expired,
        CASE,
        "after purging a lower revision",
        violations,
    )
    .await
}

async fn cursor_boundaries<Adapter>(
    adapter: &Adapter,
    owner: &Adapter::Owner,
    horizon_revision: i64,
    [earlier, later]: [i64; 2],
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    const CASE: &str = "cursor boundaries";
    if horizon_revision <= 1 || later <= horizon_revision {
        return Err(format!(
            "{CASE}: the writes left no cursor below or above the horizon"
        ));
    }
    for (cursor, label) in [
        (horizon_revision, "a cursor exactly at the horizon"),
        (horizon_revision + 1, "a cursor above the horizon"),
    ] {
        match pull(adapter, owner, cursor, CASE).await? {
            PurgeHorizonPull::Page { revisions }
                if revisions.contains(&later) || cursor >= later => {}
            PurgeHorizonPull::Page { .. } => {
                violations.push(format!("{CASE}: {label} omitted a newer live row"));
            }
            PurgeHorizonPull::ResyncRequired { .. } => {
                violations.push(format!("{CASE}: {label} required resync"));
            }
        }
    }
    match pull(adapter, owner, horizon_revision - 1, CASE).await? {
        PurgeHorizonPull::ResyncRequired {
            horizon_revision: reported,
        } if reported == horizon_revision => {}
        PurgeHorizonPull::ResyncRequired { .. } => violations.push(format!(
            "{CASE}: a cursor below the horizon reported a different horizon"
        )),
        PurgeHorizonPull::Page { .. } => violations.push(format!(
            "{CASE}: a cursor below the horizon was served instead of requiring resync"
        )),
    }
    match pull(adapter, owner, 0, CASE).await? {
        PurgeHorizonPull::Page { revisions } => {
            if revisions.contains(&horizon_revision) {
                violations.push(format!("{CASE}: a full pull returned a purged tombstone"));
            }
            if !revisions.contains(&earlier) || !revisions.contains(&later) {
                violations.push(format!("{CASE}: a full pull omitted a live row"));
            }
        }
        PurgeHorizonPull::ResyncRequired { .. } => violations.push(format!(
            "{CASE}: cursor zero required resync instead of a full pull"
        )),
    }
    Ok(())
}

async fn bounded_batches<Adapter>(
    adapter: &Adapter,
    clock: Clock,
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    const CASE: &str = "batch bounds";
    drain(adapter, clock.everything(), CASE).await?;
    let owner = step(adapter.create_owner().await, CASE, "creating an owner")?;
    let mut tombstones = Vec::with_capacity(BATCH_TOMBSTONES);
    for _ in 0..BATCH_TOMBSTONES {
        tombstones.push(step(
            adapter.write_tombstone(&owner, clock.expired()).await,
            CASE,
            "writing an expired tombstone",
        )?);
    }

    let limit = u64::from(BATCH_LIMIT.get());
    let mut remaining = BATCH_TOMBSTONES as u64;
    while remaining > 0 {
        let deleted = step(
            adapter.purge_batch(clock.cutoff(), BATCH_LIMIT).await,
            CASE,
            "purging a batch",
        )?;
        let expected = remaining.min(limit);
        if deleted != expected {
            return Err(format!(
                "{CASE}: a batch with limit {limit} removed {deleted} tombstones; expected {expected}"
            ));
        }
        remaining -= deleted;
        let purged = purged_tombstones(adapter, &owner, &tombstones, CASE).await?;
        let expected_horizon = purged.iter().copied().max();
        if horizon(adapter, &owner, CASE).await? != expected_horizon {
            violations.push(format!(
                "{CASE}: the horizon after a batch was not the greatest purged revision"
            ));
        }
    }
    let deleted = step(
        adapter.purge_batch(clock.cutoff(), BATCH_LIMIT).await,
        CASE,
        "purging an empty batch",
    )?;
    if deleted != 0 {
        violations.push(format!(
            "{CASE}: a batch after the table was drained removed {deleted} rows"
        ));
    }
    Ok(())
}

async fn purged_tombstones<Adapter>(
    adapter: &Adapter,
    owner: &Adapter::Owner,
    tombstones: &[i64],
    case: &str,
) -> Result<Vec<i64>, String>
where
    Adapter: PurgeHorizonAdapter,
{
    match pull(adapter, owner, 0, case).await? {
        PurgeHorizonPull::Page { revisions } => Ok(tombstones
            .iter()
            .copied()
            .filter(|revision| !revisions.contains(revision))
            .collect()),
        PurgeHorizonPull::ResyncRequired { .. } => Err(format!(
            "{case}: cursor zero required resync instead of a full pull"
        )),
    }
}

async fn owner_isolation<Adapter>(
    adapter: &Adapter,
    clock: Clock,
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    const CASE: &str = "owner isolation";
    drain(adapter, clock.everything(), CASE).await?;
    let busy = step(adapter.create_owner().await, CASE, "creating an owner")?;
    let quiet = step(adapter.create_owner().await, CASE, "creating an owner")?;
    let untouched = step(adapter.create_owner().await, CASE, "creating an owner")?;
    for _ in 0..BUSY_OWNER_LIVE_ROWS {
        step(
            adapter.write_live_row(&busy).await,
            CASE,
            "writing a live row",
        )?;
    }
    let busy_tombstone = step(
        adapter.write_tombstone(&busy, clock.expired()).await,
        CASE,
        "writing an expired tombstone",
    )?;
    step(
        adapter.write_live_row(&quiet).await,
        CASE,
        "writing a live row",
    )?;
    let quiet_tombstone = step(
        adapter.write_tombstone(&quiet, clock.expired()).await,
        CASE,
        "writing an expired tombstone",
    )?;
    let untouched_live = step(
        adapter.write_live_row(&untouched).await,
        CASE,
        "writing a live row",
    )?;
    let untouched_tombstone = step(
        adapter.write_tombstone(&untouched, clock.now).await,
        CASE,
        "writing a recent tombstone",
    )?;

    drain(adapter, clock.cutoff(), CASE).await?;
    expect_horizon(
        adapter,
        &busy,
        busy_tombstone,
        CASE,
        "for the first owner",
        violations,
    )
    .await?;
    expect_horizon(
        adapter,
        &quiet,
        quiet_tombstone,
        CASE,
        "for the second owner",
        violations,
    )
    .await?;
    if horizon(adapter, &untouched, CASE).await?.is_some() {
        violations.push(format!(
            "{CASE}: an owner with no expired tombstone received a horizon"
        ));
    }
    if !matches!(
        pull(adapter, &quiet, quiet_tombstone, CASE).await?,
        PurgeHorizonPull::Page { .. }
    ) {
        violations.push(format!(
            "{CASE}: a cursor at the owner's own horizon was rejected by another owner's horizon"
        ));
    }
    match pull(adapter, &untouched, untouched_live, CASE).await? {
        PurgeHorizonPull::Page { revisions } if revisions.contains(&untouched_tombstone) => {}
        PurgeHorizonPull::Page { .. } => violations.push(format!(
            "{CASE}: another owner's purge removed a tombstone that had not expired"
        )),
        PurgeHorizonPull::ResyncRequired { .. } => violations.push(format!(
            "{CASE}: an owner without a horizon was asked to resync"
        )),
    }
    Ok(())
}

async fn purge_racing_an_open_pull<Adapter>(
    adapter: &Adapter,
    clock: Clock,
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    const CASE: &str = "concurrent purge and pull";
    drain(adapter, clock.everything(), CASE).await?;
    let owner = step(adapter.create_owner().await, CASE, "creating an owner")?;
    let cursor = step(
        adapter.write_live_row(&owner).await,
        CASE,
        "writing a live row",
    )?;
    let mut tombstones = Vec::new();
    for _ in 0..2 {
        tombstones.push(step(
            adapter.write_tombstone(&owner, clock.expired()).await,
            CASE,
            "writing an expired tombstone",
        )?);
    }

    let race = race_purge_against_pull(adapter, &owner, cursor, clock.cutoff());
    let Ok(outcome) = tokio::time::timeout(RACE_DEADLINE, race).await else {
        return Err(format!(
            "{CASE}: the purge and the open pull did not finish within {RACE_DEADLINE:?}"
        ));
    };
    if !outcome.pause_reached {
        return Err(format!(
            "{CASE}: the pull never called PullPause::reached between its cursor check and its row reads"
        ));
    }
    step(outcome.purge, CASE, "purging while a pull was open")?;
    match step(outcome.pull, CASE, "pulling while a purge ran")? {
        PurgeHorizonPull::Page { revisions } => {
            if tombstones
                .iter()
                .any(|tombstone| !revisions.contains(tombstone))
            {
                violations.push(format!(
                    "{CASE}: a pull that passed its cursor check omitted a tombstone purged while it was open"
                ));
            }
        }
        PurgeHorizonPull::ResyncRequired { horizon_revision } if horizon_revision > cursor => {}
        PurgeHorizonPull::ResyncRequired { .. } => violations.push(format!(
            "{CASE}: an open pull reported a horizon at or below its cursor"
        )),
    }

    drain(adapter, clock.cutoff(), CASE).await?;
    let newest = tombstones.iter().copied().max().unwrap_or_default();
    expect_horizon(adapter, &owner, newest, CASE, "after the race", violations).await?;
    if !matches!(
        pull(adapter, &owner, cursor, CASE).await?,
        PurgeHorizonPull::ResyncRequired { .. }
    ) {
        violations.push(format!(
            "{CASE}: the cursor from before the purge was served after the horizon moved past it"
        ));
    }
    Ok(())
}

struct RaceOutcome<Error> {
    pause_reached: bool,
    purge: Result<u64, Error>,
    pull: Result<PurgeHorizonPull, Error>,
}

async fn race_purge_against_pull<Adapter>(
    adapter: &Adapter,
    owner: &Adapter::Owner,
    cursor: i64,
    cutoff: DateTime<Utc>,
) -> RaceOutcome<Adapter::Error>
where
    Adapter: PurgeHorizonAdapter,
{
    let (reached, reached_signal) = oneshot::channel();
    let (release_signal, release) = oneshot::channel();
    let pause = PullPause {
        gate: Some(PauseGate { reached, release }),
    };
    let open_pull = adapter.pull(owner, cursor, pause);
    let purge_during_pull = async {
        let pause_reached = reached_signal.await.is_ok();
        let mut purge = pin!(adapter.purge_batch(cutoff, DRAIN_LIMIT));
        let early = tokio::time::timeout(PURGE_OVERLAP, &mut purge).await;
        let _ = release_signal.send(());
        let purge = match early {
            Ok(result) => result,
            Err(_) => purge.await,
        };
        (pause_reached, purge)
    };
    let (pull, (pause_reached, purge)) = tokio::join!(open_pull, purge_during_pull);
    RaceOutcome {
        pause_reached,
        purge,
        pull,
    }
}

async fn erasure<Adapter>(
    adapter: &Adapter,
    clock: Clock,
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    const CASE: &str = "erasure";
    drain(adapter, clock.everything(), CASE).await?;
    let erased = step(adapter.create_owner().await, CASE, "creating an owner")?;
    let kept = step(adapter.create_owner().await, CASE, "creating an owner")?;
    step(
        adapter.write_tombstone(&erased, clock.expired()).await,
        CASE,
        "writing an expired tombstone",
    )?;
    step(
        adapter.write_tombstone(&erased, clock.now).await,
        CASE,
        "writing a recent tombstone",
    )?;
    let kept_tombstone = step(
        adapter.write_tombstone(&kept, clock.expired()).await,
        CASE,
        "writing an expired tombstone",
    )?;
    drain(adapter, clock.cutoff(), CASE).await?;
    if horizon(adapter, &erased, CASE).await?.is_none() {
        return Err(format!("{CASE}: the purge before erasure set no horizon"));
    }

    step(adapter.erase_owner(&erased).await, CASE, "erasing an owner")?;
    if horizon(adapter, &erased, CASE).await?.is_some() {
        violations.push(format!("{CASE}: erasing an owner left its purge horizon"));
    }
    drain(adapter, clock.everything(), CASE).await?;
    if horizon(adapter, &erased, CASE).await?.is_some() {
        violations.push(format!(
            "{CASE}: a purge after erasure recreated the erased owner's horizon"
        ));
    }
    expect_horizon(
        adapter,
        &kept,
        kept_tombstone,
        CASE,
        "for the owner that was not erased",
        violations,
    )
    .await
}

async fn drain<Adapter>(adapter: &Adapter, cutoff: DateTime<Utc>, case: &str) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    let limit = u64::from(DRAIN_LIMIT.get());
    for _ in 0..MAX_DRAIN_BATCHES {
        let deleted = step(
            adapter.purge_batch(cutoff, DRAIN_LIMIT).await,
            case,
            "purging expired tombstones",
        )?;
        if deleted > limit {
            return Err(format!(
                "{case}: a batch with limit {limit} removed {deleted} tombstones"
            ));
        }
        if deleted == 0 {
            return Ok(());
        }
    }
    Err(format!(
        "{case}: purging did not finish within {MAX_DRAIN_BATCHES} batches"
    ))
}

async fn expect_horizon<Adapter>(
    adapter: &Adapter,
    owner: &Adapter::Owner,
    expected: i64,
    case: &str,
    phase: &str,
    violations: &mut Vec<String>,
) -> CaseResult
where
    Adapter: PurgeHorizonAdapter,
{
    match horizon(adapter, owner, case).await? {
        Some(actual) if actual == expected => {}
        Some(actual) if actual < expected => violations.push(format!(
            "{case}: the horizon {phase} was below the greatest purged revision"
        )),
        Some(_) => violations.push(format!(
            "{case}: the horizon {phase} was above the greatest purged revision"
        )),
        None => violations.push(format!("{case}: no horizon was stored {phase}")),
    }
    Ok(())
}

async fn horizon<Adapter>(
    adapter: &Adapter,
    owner: &Adapter::Owner,
    case: &str,
) -> Result<Option<i64>, String>
where
    Adapter: PurgeHorizonAdapter,
{
    step(
        adapter.purge_horizon(owner).await,
        case,
        "reading the purge horizon",
    )
}

async fn pull<Adapter>(
    adapter: &Adapter,
    owner: &Adapter::Owner,
    cursor: i64,
    case: &str,
) -> Result<PurgeHorizonPull, String>
where
    Adapter: PurgeHorizonAdapter,
{
    step(
        adapter.pull(owner, cursor, PullPause::none()).await,
        case,
        "pulling",
    )
}

fn step<Value, Error>(
    result: Result<Value, Error>,
    case: &str,
    action: &str,
) -> Result<Value, String> {
    result.map_err(|_| format!("{case}: {action} failed"))
}

#[cfg(test)]
mod tests;
