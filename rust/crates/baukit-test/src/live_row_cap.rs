use std::fmt;

/// Product adapter exercised by the PostgreSQL live-row cap race check.
///
/// Bind one adapter to a clean owner, parent, or time-bucket scope. Calls to
/// [`Self::create_row`] must obtain independent database connections when they
/// overlap.
#[allow(async_fn_in_trait)]
pub trait PostgresLiveRowCapAdapter {
    /// Product row returned after creation.
    type Row;
    /// Product failure returned by row operations.
    type Error;

    /// Creates one distinct live row in the bound scope.
    async fn create_row(&self, sequence: usize) -> Result<Self::Row, Self::Error>;

    /// Updates a live row without consuming another slot.
    async fn update_row(&self, row: &Self::Row) -> Result<(), Self::Error>;

    /// Soft-deletes a row and releases its slot.
    async fn soft_delete_row(&self, row: &Self::Row) -> Result<(), Self::Error>;

    /// Counts live rows in the bound scope.
    async fn live_row_count(&self) -> Result<usize, Self::Error>;
}

/// Inputs for the PostgreSQL live-row cap race check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveRowCapConformanceCases<'a> {
    /// Maximum live rows in the bound scope.
    pub limit: usize,
    /// Stable product reason code expected on a rejected create.
    pub expected_limit_code: &'a str,
}

impl<'a> LiveRowCapConformanceCases<'a> {
    /// Creates a conformance case set.
    #[must_use]
    pub const fn new(limit: usize, expected_limit_code: &'a str) -> Self {
        Self {
            limit,
            expected_limit_code,
        }
    }
}

/// Violations found by the PostgreSQL live-row cap race check.
///
/// Messages never contain adapter errors, row values, or scope identifiers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveRowCapConformanceError {
    violations: Vec<String>,
}

impl LiveRowCapConformanceError {
    /// Returns violations in check order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for LiveRowCapConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "PostgreSQL live-row cap conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LiveRowCapConformanceError {}

/// Races two creates for the last slot, then checks update and soft-delete behavior.
///
/// The adapter must start with no live rows. `extract_limit_code` reads the
/// product's stable code without exposing the rest of its error. The check
/// fills `limit - 1` slots, polls both final creates concurrently, and requires
/// exactly one success. It then updates at capacity, soft-deletes one row, and
/// creates a replacement.
pub async fn check_postgres_live_row_cap_conformance<Adapter, ExtractLimitCode>(
    adapter: &Adapter,
    cases: LiveRowCapConformanceCases<'_>,
    extract_limit_code: ExtractLimitCode,
) -> Result<(), LiveRowCapConformanceError>
where
    Adapter: PostgresLiveRowCapAdapter,
    ExtractLimitCode: for<'error> Fn(&'error Adapter::Error) -> Option<&'error str>,
{
    validate_cases(cases)?;
    require_initially_empty(adapter).await?;

    let mut rows = fill_before_last_slot(adapter, cases.limit).await?;
    let first_sequence = cases.limit - 1;
    let second_sequence = cases.limit;
    let (first, second) = tokio::join!(
        adapter.create_row(first_sequence),
        adapter.create_row(second_sequence)
    );

    let mut violations = Vec::new();
    let mut accepted = 0;
    let mut rejected = 0;
    for result in [first, second] {
        match result {
            Ok(row) => {
                accepted += 1;
                rows.push(row);
            }
            Err(error) => {
                rejected += 1;
                if extract_limit_code(&error) != Some(cases.expected_limit_code) {
                    violations.push(
                        "a raced create rejection did not carry the expected stable limit code"
                            .to_owned(),
                    );
                }
            }
        }
    }
    if accepted != 1 || rejected != 1 {
        violations.push(format!(
            "the last-slot race accepted {accepted} creates and rejected {rejected}; expected one of each"
        ));
    }
    check_count(
        adapter,
        cases.limit,
        "after the last-slot race",
        &mut violations,
    )
    .await;

    let update_and_delete_row = rows.last();
    match update_and_delete_row {
        Some(row) if adapter.update_row(row).await.is_err() => {
            violations.push("updating a live row at capacity failed".to_owned());
        }
        None => violations.push("the check had no live row to update".to_owned()),
        Some(_) => {}
    }

    if let Some(row) = update_and_delete_row {
        if adapter.soft_delete_row(row).await.is_err() {
            violations.push("soft-deleting a live row at capacity failed".to_owned());
        } else {
            check_count(
                adapter,
                cases.limit - 1,
                "after soft deletion",
                &mut violations,
            )
            .await;
            if adapter.create_row(cases.limit + 1).await.is_err() {
                violations.push("creating a replacement after soft deletion failed".to_owned());
            }
            check_count(
                adapter,
                cases.limit,
                "after creating the replacement",
                &mut violations,
            )
            .await;
        }
    }

    finish(violations)
}

/// Panics when PostgreSQL live-row cap behavior fails the race check.
///
/// # Panics
///
/// Panics with every conformance violation.
pub async fn assert_postgres_live_row_cap_conformance<Adapter, ExtractLimitCode>(
    adapter: &Adapter,
    cases: LiveRowCapConformanceCases<'_>,
    extract_limit_code: ExtractLimitCode,
) where
    Adapter: PostgresLiveRowCapAdapter,
    ExtractLimitCode: for<'error> Fn(&'error Adapter::Error) -> Option<&'error str>,
{
    if let Err(error) =
        check_postgres_live_row_cap_conformance(adapter, cases, extract_limit_code).await
    {
        panic!("{error}");
    }
}

fn validate_cases(cases: LiveRowCapConformanceCases<'_>) -> Result<(), LiveRowCapConformanceError> {
    let mut violations = Vec::new();
    if cases.limit == 0 {
        violations.push("the live-row limit must be at least 1".to_owned());
    }
    if cases.limit == usize::MAX {
        violations.push("the live-row limit must be less than usize::MAX".to_owned());
    }
    if cases.expected_limit_code.trim().is_empty() {
        violations.push("the expected stable limit code must not be empty".to_owned());
    }
    finish(violations)
}

async fn require_initially_empty<Adapter>(
    adapter: &Adapter,
) -> Result<(), LiveRowCapConformanceError>
where
    Adapter: PostgresLiveRowCapAdapter,
{
    match adapter.live_row_count().await {
        Ok(0) => Ok(()),
        Ok(count) => failure(format!(
            "the bound scope started with {count} live rows; expected zero"
        )),
        Err(_) => failure("counting the initially empty scope failed"),
    }
}

async fn fill_before_last_slot<Adapter>(
    adapter: &Adapter,
    limit: usize,
) -> Result<Vec<Adapter::Row>, LiveRowCapConformanceError>
where
    Adapter: PostgresLiveRowCapAdapter,
{
    let mut rows = Vec::with_capacity(limit);
    for sequence in 0..limit - 1 {
        match adapter.create_row(sequence).await {
            Ok(row) => rows.push(row),
            Err(_) => {
                return failure(format!(
                    "creating live row {} of {limit} failed before the last slot",
                    sequence + 1
                ));
            }
        }
    }
    Ok(rows)
}

async fn check_count<Adapter>(
    adapter: &Adapter,
    expected: usize,
    phase: &str,
    violations: &mut Vec<String>,
) where
    Adapter: PostgresLiveRowCapAdapter,
{
    match adapter.live_row_count().await {
        Ok(actual) if actual == expected => {}
        Ok(actual) => violations.push(format!(
            "the live-row count {phase} was {actual}; expected {expected}"
        )),
        Err(_) => violations.push(format!("counting live rows {phase} failed")),
    }
}

fn finish(violations: Vec<String>) -> Result<(), LiveRowCapConformanceError> {
    if violations.is_empty() {
        Ok(())
    } else {
        Err(LiveRowCapConformanceError { violations })
    }
}

fn failure<Output>(message: impl Into<String>) -> Result<Output, LiveRowCapConformanceError> {
    Err(LiveRowCapConformanceError {
        violations: vec![message.into()],
    })
}

#[cfg(all(test, feature = "sqlx-postgres"))]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use sqlx::PgPool;

    use super::*;
    use crate::{LiveRowLimitAdapter, check_update_at_capacity, start_postgres};

    const LIMIT: usize = 2;
    const LIMIT_CODE: &str = "row_cap_per_scope";
    const EMPIRICAL_RACE_RUNS: i64 = 16;
    const MAX_SERIALIZATION_ATTEMPTS: usize = 3;
    const RACE_OVERLAP_SECONDS: f64 = 0.02;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Strategy {
        RowLock,
        Serializable,
        Counter,
        Constraint,
    }

    impl Strategy {
        const ALL: [Self; 4] = [
            Self::RowLock,
            Self::Serializable,
            Self::Counter,
            Self::Constraint,
        ];

        const fn name(self) -> &'static str {
            match self {
                Self::RowLock => "row_lock",
                Self::Serializable => "serializable",
                Self::Counter => "counter",
                Self::Constraint => "constraint",
            }
        }
    }

    #[derive(Debug)]
    struct TestRow {
        id: i64,
    }

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("live-row cap reached")]
        Limit,
        #[error("database operation failed: {0}")]
        Database(sqlx::Error),
    }

    impl From<sqlx::Error> for TestError {
        fn from(error: sqlx::Error) -> Self {
            Self::Database(error)
        }
    }

    #[derive(Default)]
    struct Observations {
        row_lock_limit_reads: AtomicUsize,
        serialization_failures: AtomicUsize,
        counter_misses: AtomicUsize,
        unique_conflicts: AtomicUsize,
    }

    struct TestAdapter {
        pool: PgPool,
        strategy: Strategy,
        scope_id: i64,
        observations: Arc<Observations>,
        race_barrier: Arc<tokio::sync::Barrier>,
    }

    impl TestAdapter {
        async fn insert(
            transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
            strategy: Strategy,
            scope_id: i64,
            sequence: usize,
            slot: Option<i32>,
        ) -> Result<TestRow, sqlx::Error> {
            let id =
                i64::try_from(sequence).map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
            sqlx::query(
                "INSERT INTO live_row_cap_rows (strategy, scope_id, id, slot, value) \
                 VALUES ($1, $2, $3, $4, 'created')",
            )
            .bind(strategy.name())
            .bind(scope_id)
            .bind(id)
            .bind(slot)
            .execute(&mut **transaction)
            .await?;
            Ok(TestRow { id })
        }

        async fn pause_for_overlap(
            transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        ) -> Result<(), sqlx::Error> {
            sqlx::query("SELECT pg_sleep($1)")
                .bind(RACE_OVERLAP_SECONDS)
                .execute(&mut **transaction)
                .await?;
            Ok(())
        }

        async fn create_with_row_lock(&self, sequence: usize) -> Result<TestRow, TestError> {
            let mut transaction = self.pool.begin().await?;
            sqlx::query(
                "SELECT scope_id FROM live_row_cap_scopes \
                 WHERE strategy = $1 AND scope_id = $2 FOR UPDATE",
            )
            .bind(self.strategy.name())
            .bind(self.scope_id)
            .fetch_one(&mut *transaction)
            .await?;
            let count = count_rows(&mut transaction, self.strategy, self.scope_id).await?;
            if count >= LIMIT as i64 {
                self.observations
                    .row_lock_limit_reads
                    .fetch_add(1, Ordering::Relaxed);
                return Err(TestError::Limit);
            }
            Self::pause_for_overlap(&mut transaction).await?;
            let row = Self::insert(
                &mut transaction,
                self.strategy,
                self.scope_id,
                sequence,
                None,
            )
            .await?;
            transaction.commit().await?;
            Ok(row)
        }

        async fn create_serializable(&self, sequence: usize) -> Result<TestRow, TestError> {
            for attempt in 0..MAX_SERIALIZATION_ATTEMPTS {
                match self.serializable_attempt(sequence).await {
                    Err(TestError::Database(error)) if has_database_code(&error, "40001") => {
                        self.observations
                            .serialization_failures
                            .fetch_add(1, Ordering::Relaxed);
                        if attempt + 1 == MAX_SERIALIZATION_ATTEMPTS {
                            return Err(TestError::Database(error));
                        }
                    }
                    result => return result,
                }
            }
            unreachable!("the serialization attempt loop always returns")
        }

        async fn serializable_attempt(&self, sequence: usize) -> Result<TestRow, TestError> {
            let mut transaction = self.pool.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                .execute(&mut *transaction)
                .await?;
            let count = count_rows(&mut transaction, self.strategy, self.scope_id).await?;
            if count >= LIMIT as i64 {
                return Err(TestError::Limit);
            }
            Self::pause_for_overlap(&mut transaction).await?;
            let row = Self::insert(
                &mut transaction,
                self.strategy,
                self.scope_id,
                sequence,
                None,
            )
            .await?;
            transaction.commit().await?;
            Ok(row)
        }

        async fn create_with_counter(&self, sequence: usize) -> Result<TestRow, TestError> {
            let mut transaction = self.pool.begin().await?;
            let reserved = sqlx::query_scalar::<_, i32>(
                "UPDATE live_row_cap_scopes SET live_count = live_count + 1 \
                 WHERE strategy = $1 AND scope_id = $2 AND live_count < $3 \
                 RETURNING live_count",
            )
            .bind(self.strategy.name())
            .bind(self.scope_id)
            .bind(LIMIT as i32)
            .fetch_optional(&mut *transaction)
            .await?;
            if reserved.is_none() {
                self.observations
                    .counter_misses
                    .fetch_add(1, Ordering::Relaxed);
                return Err(TestError::Limit);
            }
            let row = Self::insert(
                &mut transaction,
                self.strategy,
                self.scope_id,
                sequence,
                None,
            )
            .await?;
            transaction.commit().await?;
            Ok(row)
        }

        async fn create_with_constraint(&self, sequence: usize) -> Result<TestRow, TestError> {
            let mut transaction = self.pool.begin().await?;
            let slot = sqlx::query_scalar::<_, i32>(
                "SELECT candidate FROM generate_series(1, $3) AS candidate \
                 WHERE NOT EXISTS ( \
                     SELECT 1 FROM live_row_cap_rows \
                     WHERE strategy = $1 AND scope_id = $2 \
                       AND slot = candidate AND deleted_at IS NULL \
                 ) ORDER BY candidate LIMIT 1",
            )
            .bind(self.strategy.name())
            .bind(self.scope_id)
            .bind(LIMIT as i32)
            .fetch_optional(&mut *transaction)
            .await?;
            let Some(slot) = slot else {
                return Err(TestError::Limit);
            };
            Self::pause_for_overlap(&mut transaction).await?;
            match Self::insert(
                &mut transaction,
                self.strategy,
                self.scope_id,
                sequence,
                Some(slot),
            )
            .await
            {
                Ok(row) => {
                    transaction.commit().await?;
                    Ok(row)
                }
                Err(error) if unique_slot_conflict(&error) => {
                    self.observations
                        .unique_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    Err(TestError::Limit)
                }
                Err(error) => Err(TestError::Database(error)),
            }
        }
    }

    impl PostgresLiveRowCapAdapter for TestAdapter {
        type Row = TestRow;
        type Error = TestError;

        async fn create_row(&self, sequence: usize) -> Result<Self::Row, Self::Error> {
            if sequence == LIMIT - 1 || sequence == LIMIT {
                self.race_barrier.wait().await;
            }
            match self.strategy {
                Strategy::RowLock => self.create_with_row_lock(sequence).await,
                Strategy::Serializable => self.create_serializable(sequence).await,
                Strategy::Counter => self.create_with_counter(sequence).await,
                Strategy::Constraint => self.create_with_constraint(sequence).await,
            }
        }

        async fn update_row(&self, row: &Self::Row) -> Result<(), Self::Error> {
            sqlx::query(
                "UPDATE live_row_cap_rows SET value = 'updated' \
                 WHERE strategy = $1 AND scope_id = $2 AND id = $3 AND deleted_at IS NULL",
            )
            .bind(self.strategy.name())
            .bind(self.scope_id)
            .bind(row.id)
            .execute(&self.pool)
            .await?;
            Ok(())
        }

        async fn soft_delete_row(&self, row: &Self::Row) -> Result<(), Self::Error> {
            let mut transaction = self.pool.begin().await?;
            if self.strategy == Strategy::RowLock {
                sqlx::query(
                    "SELECT scope_id FROM live_row_cap_scopes \
                     WHERE strategy = $1 AND scope_id = $2 FOR UPDATE",
                )
                .bind(self.strategy.name())
                .bind(self.scope_id)
                .fetch_one(&mut *transaction)
                .await?;
            }
            if self.strategy == Strategy::Serializable {
                sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                    .execute(&mut *transaction)
                    .await?;
            }
            let deleted = sqlx::query(
                "UPDATE live_row_cap_rows SET deleted_at = now() \
                 WHERE strategy = $1 AND scope_id = $2 AND id = $3 AND deleted_at IS NULL \
                 RETURNING id",
            )
            .bind(self.strategy.name())
            .bind(self.scope_id)
            .bind(row.id)
            .fetch_optional(&mut *transaction)
            .await?;
            if deleted.is_none() {
                return Err(TestError::Limit);
            }
            if self.strategy == Strategy::Counter {
                sqlx::query(
                    "UPDATE live_row_cap_scopes SET live_count = live_count - 1 \
                     WHERE strategy = $1 AND scope_id = $2",
                )
                .bind(self.strategy.name())
                .bind(self.scope_id)
                .execute(&mut *transaction)
                .await?;
            }
            transaction.commit().await?;
            Ok(())
        }

        async fn live_row_count(&self) -> Result<usize, Self::Error> {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM live_row_cap_rows \
                 WHERE strategy = $1 AND scope_id = $2 AND deleted_at IS NULL",
            )
            .bind(self.strategy.name())
            .bind(self.scope_id)
            .fetch_one(&self.pool)
            .await?;
            usize::try_from(count)
                .map_err(|error| TestError::Database(sqlx::Error::Decode(Box::new(error))))
        }
    }

    async fn count_rows(
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        strategy: Strategy,
        scope_id: i64,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM live_row_cap_rows \
             WHERE strategy = $1 AND scope_id = $2 AND deleted_at IS NULL",
        )
        .bind(strategy.name())
        .bind(scope_id)
        .fetch_one(&mut **transaction)
        .await
    }

    fn has_database_code(error: &sqlx::Error, expected: &str) -> bool {
        match error {
            sqlx::Error::Database(database) => database.code().as_deref() == Some(expected),
            _ => false,
        }
    }

    fn unique_slot_conflict(error: &sqlx::Error) -> bool {
        match error {
            sqlx::Error::Database(database) => {
                database.code().as_deref() == Some("23505")
                    && database.constraint() == Some("live_row_cap_rows_live_slot_idx")
            }
            _ => false,
        }
    }

    async fn create_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(
            "CREATE TABLE live_row_cap_scopes ( \
                 strategy TEXT NOT NULL, \
                 scope_id BIGINT NOT NULL, \
                 live_count INTEGER NOT NULL DEFAULT 0 CHECK (live_count >= 0), \
                 PRIMARY KEY (strategy, scope_id) \
             ); \
             CREATE TABLE live_row_cap_rows ( \
                 strategy TEXT NOT NULL, \
                 scope_id BIGINT NOT NULL, \
                 id BIGINT NOT NULL, \
                 slot INTEGER CHECK (slot IS NULL OR slot BETWEEN 1 AND 2), \
                 value TEXT NOT NULL, \
                 deleted_at TIMESTAMPTZ, \
                 CHECK (strategy <> 'constraint' OR deleted_at IS NOT NULL OR slot IS NOT NULL), \
                 PRIMARY KEY (strategy, scope_id, id), \
                 FOREIGN KEY (strategy, scope_id) \
                     REFERENCES live_row_cap_scopes(strategy, scope_id) \
             ); \
             CREATE INDEX live_row_cap_rows_count_idx \
                 ON live_row_cap_rows (strategy, scope_id) WHERE deleted_at IS NULL; \
             CREATE UNIQUE INDEX live_row_cap_rows_live_slot_idx \
                 ON live_row_cap_rows (strategy, scope_id, slot) \
                 WHERE deleted_at IS NULL AND slot IS NOT NULL;",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn create_scope(pool: &PgPool, strategy: &str, scope_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO live_row_cap_scopes (strategy, scope_id) VALUES ($1, $2)")
            .bind(strategy)
            .bind(scope_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn run_naive_race(pool: &PgPool, scope_id: i64) -> Result<usize, TestError> {
        create_scope(pool, "naive", scope_id).await?;
        sqlx::query(
            "INSERT INTO live_row_cap_rows (strategy, scope_id, id, value) \
             VALUES ('naive', $1, 0, 'prefill')",
        )
        .bind(scope_id)
        .execute(pool)
        .await?;

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let create = |id: i64| {
            let barrier = barrier.clone();
            async move {
                let mut transaction = pool.begin().await?;
                let count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM live_row_cap_rows \
                     WHERE strategy = 'naive' AND scope_id = $1 AND deleted_at IS NULL",
                )
                .bind(scope_id)
                .fetch_one(&mut *transaction)
                .await?;
                if count >= LIMIT as i64 {
                    return Err(TestError::Limit);
                }
                barrier.wait().await;
                sqlx::query(
                    "INSERT INTO live_row_cap_rows (strategy, scope_id, id, value) \
                     VALUES ('naive', $1, $2, 'raced')",
                )
                .bind(scope_id)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok::<(), TestError>(())
            }
        };
        let (first, second) = tokio::join!(create(1), create(2));
        Ok(usize::from(first.is_ok()) + usize::from(second.is_ok()))
    }

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn compares_live_row_cap_methods_on_postgres() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = start_postgres().await?;
        let pool = PgPool::connect(fixture.connection_url()).await?;
        create_schema(&pool).await?;
        let observations = Arc::new(Observations::default());

        for run in 0..EMPIRICAL_RACE_RUNS {
            assert_eq!(run_naive_race(&pool, run).await?, 2);
            for (strategy_index, strategy) in Strategy::ALL.into_iter().enumerate() {
                let scope_id = (i64::try_from(strategy_index)? + 1) * EMPIRICAL_RACE_RUNS + run;
                create_scope(&pool, strategy.name(), scope_id).await?;
                let adapter = TestAdapter {
                    pool: pool.clone(),
                    strategy,
                    scope_id,
                    observations: observations.clone(),
                    race_barrier: Arc::new(tokio::sync::Barrier::new(2)),
                };
                check_postgres_live_row_cap_conformance(
                    &adapter,
                    LiveRowCapConformanceCases::new(LIMIT, LIMIT_CODE),
                    |error| match error {
                        TestError::Limit => Some(LIMIT_CODE),
                        TestError::Database(_) => None,
                    },
                )
                .await?;
            }
        }

        let expected = usize::try_from(EMPIRICAL_RACE_RUNS)?;
        assert_eq!(
            observations.row_lock_limit_reads.load(Ordering::Relaxed),
            expected
        );
        assert_eq!(
            observations.serialization_failures.load(Ordering::Relaxed),
            expected
        );
        assert_eq!(
            observations.counter_misses.load(Ordering::Relaxed),
            expected
        );
        assert_eq!(
            observations.unique_conflicts.load(Ordering::Relaxed),
            expected
        );
        pool.close().await;
        Ok(())
    }

    struct PreviousAdapter {
        rows: Mutex<Vec<bool>>,
    }

    impl LiveRowLimitAdapter for PreviousAdapter {
        type Row = usize;
        type Error = &'static str;

        async fn create_row(&mut self, _sequence: usize) -> Result<Self::Row, Self::Error> {
            let rows = self.rows.get_mut().expect("mutex is not poisoned");
            if rows.iter().filter(|deleted| !**deleted).count() >= LIMIT {
                return Err(LIMIT_CODE);
            }
            rows.push(false);
            Ok(rows.len() - 1)
        }

        async fn update_row(&mut self, _row: &Self::Row) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn soft_delete_row(&mut self, row: &Self::Row) -> Result<(), Self::Error> {
            self.rows.get_mut().expect("mutex is not poisoned")[*row] = true;
            Ok(())
        }
    }

    #[tokio::test]
    async fn previous_sequential_live_row_helper_remains_available() {
        let mut adapter = PreviousAdapter {
            rows: Mutex::new(Vec::new()),
        };
        check_update_at_capacity(&mut adapter, LIMIT)
            .await
            .expect("previous helper should pass");
    }

    #[tokio::test]
    async fn invalid_cases_fail_before_calling_the_adapter() {
        struct UnusedAdapter;

        impl PostgresLiveRowCapAdapter for UnusedAdapter {
            type Row = ();
            type Error = ();

            async fn create_row(&self, _sequence: usize) -> Result<Self::Row, Self::Error> {
                panic!("adapter must not be called")
            }

            async fn update_row(&self, _row: &Self::Row) -> Result<(), Self::Error> {
                panic!("adapter must not be called")
            }

            async fn soft_delete_row(&self, _row: &Self::Row) -> Result<(), Self::Error> {
                panic!("adapter must not be called")
            }

            async fn live_row_count(&self) -> Result<usize, Self::Error> {
                panic!("adapter must not be called")
            }
        }

        let error = check_postgres_live_row_cap_conformance(
            &UnusedAdapter,
            LiveRowCapConformanceCases::new(0, ""),
            |_| None,
        )
        .await
        .expect_err("invalid cases should fail");
        assert_eq!(error.violations().len(), 2);
    }
}
