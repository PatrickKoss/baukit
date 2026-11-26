#![cfg(test)]
//! P-10: jobs lifecycle as a random state machine on PostgreSQL.
//!
//! Several runner slots share one worker id. Actions are claim, complete,
//! fail, expire (the clock passes every lease), cancel, and stall (a slot
//! keeps a lease it never acts on). Invariant: a job only reaches
//! `succeeded` through a completion by its latest attempt.
use std::{collections::BTreeMap, error::Error, path::PathBuf, time::Duration};

use baukit_jobs::{JobLease, JobStore as _, NewJob, PostgresJobStore};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const WORKER: &str = "shared-worker";
const KIND: &str = "machine.job";
const SLOTS: usize = 3;
const JOBS: usize = 4;
const STEPS: usize = 60;
const SEEDS: u64 = 8;
const LEASE: Duration = Duration::from_secs(30);

struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(bound).unwrap_or(1)).unwrap_or(0)
    }
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn succeeded_jobs_were_completed_by_their_latest_attempt() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    for seed in 1..=SEEDS {
        run(&pool, &store, seed).await?;
    }
    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn run(pool: &PgPool, store: &PostgresJobStore, seed: u64) -> Result<(), Box<dyn Error>> {
    sqlx::query("DELETE FROM job_outbox").execute(pool).await?;
    for _ in 0..JOBS {
        store.enqueue(NewJob::new(KIND, json!({}), 3)).await?;
    }
    let mut random = Random(seed);
    let mut now = Timestamp::now();
    let mut slots: Vec<Option<JobLease>> = vec![None; SLOTS];
    let mut completed_by: BTreeMap<Uuid, u32> = BTreeMap::new();
    for _ in 0..STEPS {
        let slot = random.below(SLOTS);
        match random.below(7) {
            0 | 1 => {
                if slots[slot].is_none()
                    && let Some(job) = store.claim(WORKER, &[KIND], now, LEASE).await?
                {
                    slots[slot] = Some(JobLease::of(&job, WORKER));
                }
            }
            2 => {
                if let Some(lease) = slots[slot].take() {
                    let latest = attempts(pool, lease.job_id).await?;
                    if store.complete(&lease, now).await? {
                        assert_eq!(lease.attempt, latest, "seed {seed}: stale completion");
                        completed_by.insert(lease.job_id, lease.attempt);
                    }
                }
            }
            3 => {
                if let Some(lease) = slots[slot].take() {
                    store
                        .record_failure(&lease, random.below(2) == 0, now, "failed", now)
                        .await?;
                }
            }
            4 => now += SignedDuration::from_mins(1),
            5 => {
                let jobs = job_ids(pool).await?;
                let job = jobs[random.below(jobs.len())];
                store.request_cancellation(job, now).await?;
                if let Some(lease) = slots.iter().flatten().find(|lease| lease.job_id == job)
                    && store.cancellation_requested(job).await?
                {
                    store.cancel(lease, now).await?;
                }
            }
            _ => now += SignedDuration::from_secs(1),
        }
    }
    assert_succeeded_by_latest(pool, &completed_by, seed).await
}

async fn assert_succeeded_by_latest(
    pool: &PgPool,
    completed_by: &BTreeMap<Uuid, u32>,
    seed: u64,
) -> Result<(), Box<dyn Error>> {
    let succeeded: Vec<(Uuid, i32)> =
        sqlx::query_as("SELECT id, attempts FROM job_outbox WHERE status = 'succeeded'")
            .fetch_all(pool)
            .await?;
    for (id, attempts) in succeeded {
        assert_eq!(
            completed_by.get(&id).copied(),
            Some(u32::try_from(attempts)?),
            "seed {seed}: job {id} succeeded without its latest attempt completing it"
        );
    }
    Ok(())
}

async fn attempts(pool: &PgPool, id: Uuid) -> Result<u32, Box<dyn Error>> {
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM job_outbox WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(u32::try_from(attempts)?)
}

async fn job_ids(pool: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM job_outbox ORDER BY id")
        .fetch_all(pool)
        .await
}
