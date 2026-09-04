//! PostgreSQL inbox reliability conformance checks.

use std::{fmt, future::Future, pin::Pin};

use serde_json::Value;

/// A boxed operation returned by [`PostgresInboxPort`].
pub type InboxFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// The explicit idempotency scope for one inbound event.
///
/// The database uniqueness constraint must cover all three fields. An event ID
/// alone is not an owner-safe idempotency key.
#[derive(Clone, Eq, PartialEq)]
pub struct InboxScope {
    /// Opaque product owner or tenant key.
    pub owner_key: String,
    /// Stable source or connection key.
    pub source: String,
    /// Stable event identifier within the source.
    pub event_id: String,
}

impl InboxScope {
    /// Creates an inbox scope without interpreting any product identifier.
    pub fn new(
        owner_key: impl Into<String>,
        source: impl Into<String>,
        event_id: impl Into<String>,
    ) -> Self {
        Self {
            owner_key: owner_key.into(),
            source: source.into(),
            event_id: event_id.into(),
        }
    }
}

/// One inbound delivery passed to a product adapter.
#[derive(Clone, Eq, PartialEq)]
pub struct InboxDelivery {
    /// Idempotency scope used for lookup and insertion.
    pub scope: InboxScope,
    /// Product-defined event payload.
    pub payload: Value,
}

impl InboxDelivery {
    /// Creates a delivery from its scope and product payload.
    pub const fn new(scope: InboxScope, payload: Value) -> Self {
        Self { scope, payload }
    }

    fn with_scope(&self, scope: InboxScope) -> Self {
        Self {
            scope,
            payload: self.payload.clone(),
        }
    }
}

/// Whether a call processed an event or replayed its committed outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InboxDisposition {
    /// This call committed the first delivery.
    FirstDelivery,
    /// This call returned the outcome stored by an earlier delivery.
    Replay,
}

/// Result returned after a committed first delivery or replay.
#[derive(Clone, Eq, PartialEq)]
pub struct InboxReceipt {
    /// Whether this call performed the work.
    pub disposition: InboxDisposition,
    /// Product-defined durable outcome returned to the sender.
    pub outcome: Value,
}

/// Committed state for one inbox scope.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct InboxState {
    /// Number of inbox rows for the scope.
    pub inbox_rows: u64,
    /// Number of committed domain effects for the scope.
    pub domain_effects: u64,
    /// Number of committed outbox messages for the scope.
    pub outbox_messages: u64,
    /// Outcome stored on the inbox row, if any.
    pub stored_outcome: Option<Value>,
}

/// Injected failure point used by the conformance runner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InboxFault {
    /// Fail after inserting the inbox row and before the domain write.
    AfterInboxInsert,
    /// Fail at the domain write.
    DomainWrite,
    /// Fail at the outbox write after the domain write.
    OutboxWrite,
}

/// Product adapter used by the PostgreSQL inbox conformance runner.
///
/// `deliver` must run the inbox insert, domain write, outcome update, and
/// outbox write in one PostgreSQL transaction. The fault argument is test-only
/// instrumentation at the named point. `discard_transient_state` must clear
/// cached outcomes or recreate process-local components without deleting rows.
pub trait PostgresInboxPort: Send + Sync {
    /// Product adapter error. The conformance runner never formats it.
    type Error: Send + Sync + 'static;

    /// Delivers one event, optionally failing at a transaction boundary.
    fn deliver<'a>(
        &'a self,
        delivery: InboxDelivery,
        fault: Option<InboxFault>,
    ) -> InboxFuture<'a, InboxReceipt, Self::Error>;

    /// Reads committed counts and the stored outcome for one scope.
    fn state<'a>(&'a self, scope: &'a InboxScope) -> InboxFuture<'a, InboxState, Self::Error>;

    /// Clears process-local outcome state while preserving PostgreSQL rows.
    fn discard_transient_state<'a>(&'a self) -> InboxFuture<'a, (), Self::Error>;
}

/// Inputs used to exercise independent inbox cases.
#[derive(Clone, Eq, PartialEq)]
pub struct InboxConformanceCases {
    delivery: InboxDelivery,
    other_owner_key: String,
    other_source: String,
    concurrent_event_id: String,
    after_insert_event_id: String,
    domain_failure_event_id: String,
    outbox_failure_event_id: String,
}

impl InboxConformanceCases {
    /// Creates cases from one representative product delivery.
    ///
    /// Default case event IDs add a short suffix to the supplied event ID.
    /// Use [`with_case_event_ids`](Self::with_case_event_ids) when a product
    /// requires a fixed identifier format such as UUID.
    pub fn new(
        delivery: InboxDelivery,
        other_owner_key: impl Into<String>,
        other_source: impl Into<String>,
    ) -> Self {
        let event_id = delivery.scope.event_id.clone();
        Self {
            delivery,
            other_owner_key: other_owner_key.into(),
            other_source: other_source.into(),
            concurrent_event_id: format!("{event_id}-concurrent"),
            after_insert_event_id: format!("{event_id}-after-insert"),
            domain_failure_event_id: format!("{event_id}-domain-failure"),
            outbox_failure_event_id: format!("{event_id}-outbox-failure"),
        }
    }

    /// Replaces event IDs used by the concurrent and failure cases.
    #[must_use]
    pub fn with_case_event_ids(
        mut self,
        concurrent: impl Into<String>,
        after_insert: impl Into<String>,
        domain_failure: impl Into<String>,
        outbox_failure: impl Into<String>,
    ) -> Self {
        self.concurrent_event_id = concurrent.into();
        self.after_insert_event_id = after_insert.into();
        self.domain_failure_event_id = domain_failure.into();
        self.outbox_failure_event_id = outbox_failure.into();
        self
    }
}

/// Violations found while exercising a PostgreSQL inbox adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxConformanceError {
    violations: Vec<String>,
}

impl InboxConformanceError {
    /// Returns violations in check order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for InboxConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "PostgreSQL inbox conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for InboxConformanceError {}

/// Checks first delivery, replay, concurrency, rollback, isolation, and durability.
pub async fn check_postgres_inbox_conformance<Port>(
    port: &Port,
    cases: InboxConformanceCases,
) -> Result<(), InboxConformanceError>
where
    Port: PostgresInboxPort,
{
    let mut violations = Vec::new();
    let delivery = cases.delivery;

    let first_outcome = check_first_delivery(port, &delivery, &mut violations).await;
    if let Some(outcome) = &first_outcome {
        check_replay(port, &delivery, outcome, "exact replay", &mut violations).await;
    }

    let concurrent = delivery.with_scope(InboxScope {
        event_id: cases.concurrent_event_id,
        ..delivery.scope.clone()
    });
    check_concurrent_replay(port, &concurrent, &mut violations).await;

    for (fault, event_id, case) in [
        (
            InboxFault::AfterInboxInsert,
            cases.after_insert_event_id,
            "rollback after inbox insert",
        ),
        (
            InboxFault::DomainWrite,
            cases.domain_failure_event_id,
            "domain failure",
        ),
        (
            InboxFault::OutboxWrite,
            cases.outbox_failure_event_id,
            "outbox failure",
        ),
    ] {
        let fault_delivery = delivery.with_scope(InboxScope {
            event_id,
            ..delivery.scope.clone()
        });
        check_fault_rollback(port, &fault_delivery, fault, case, &mut violations).await;
    }

    let other_owner = delivery.with_scope(InboxScope {
        owner_key: cases.other_owner_key,
        ..delivery.scope.clone()
    });
    check_first_delivery(port, &other_owner, &mut violations).await;

    let other_source = delivery.with_scope(InboxScope {
        source: cases.other_source,
        ..delivery.scope.clone()
    });
    check_first_delivery(port, &other_source, &mut violations).await;

    if port.discard_transient_state().await.is_err() {
        violations.push("could not discard process-local outcome state".into());
    } else if let Some(outcome) = &first_outcome {
        check_replay(
            port,
            &delivery,
            outcome,
            "durable outcome replay",
            &mut violations,
        )
        .await;
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(InboxConformanceError { violations })
    }
}

/// Panics when a PostgreSQL inbox adapter violates the contract.
///
/// # Panics
///
/// Panics with every conformance violation.
pub async fn assert_postgres_inbox_conformance<Port>(port: &Port, cases: InboxConformanceCases)
where
    Port: PostgresInboxPort,
{
    if let Err(error) = check_postgres_inbox_conformance(port, cases).await {
        panic!("{error}");
    }
}

async fn check_first_delivery<Port: PostgresInboxPort>(
    port: &Port,
    delivery: &InboxDelivery,
    violations: &mut Vec<String>,
) -> Option<Value> {
    match port.deliver(delivery.clone(), None).await {
        Ok(receipt) if receipt.disposition == InboxDisposition::FirstDelivery => {
            check_committed_state(
                port,
                &delivery.scope,
                &receipt.outcome,
                "first delivery",
                violations,
            )
            .await;
            Some(receipt.outcome)
        }
        Ok(_) => {
            violations.push("first delivery returned the wrong disposition".into());
            None
        }
        Err(_) => {
            violations.push("first delivery failed".into());
            None
        }
    }
}

async fn check_replay<Port: PostgresInboxPort>(
    port: &Port,
    delivery: &InboxDelivery,
    expected_outcome: &Value,
    case: &str,
    violations: &mut Vec<String>,
) {
    match port.deliver(delivery.clone(), None).await {
        Ok(receipt)
            if receipt.disposition == InboxDisposition::Replay
                && &receipt.outcome == expected_outcome => {}
        Ok(_) => violations.push(format!("{case} returned the wrong disposition or outcome")),
        Err(_) => violations.push(format!("{case} failed")),
    }
    check_committed_state(port, &delivery.scope, expected_outcome, case, violations).await;
}

async fn check_concurrent_replay<Port: PostgresInboxPort>(
    port: &Port,
    delivery: &InboxDelivery,
    violations: &mut Vec<String>,
) {
    let (first, second) = tokio::join!(
        port.deliver(delivery.clone(), None),
        port.deliver(delivery.clone(), None)
    );
    let mut first_outcome = None;
    let mut replay_outcome = None;
    for result in [first, second] {
        match result {
            Ok(receipt) if receipt.disposition == InboxDisposition::FirstDelivery => {
                first_outcome = Some(receipt.outcome);
            }
            Ok(receipt) if receipt.disposition == InboxDisposition::Replay => {
                replay_outcome = Some(receipt.outcome);
            }
            Ok(_) | Err(_) => {}
        }
    }
    if first_outcome.is_none()
        || replay_outcome.is_none()
        || first_outcome.as_ref() != replay_outcome.as_ref()
    {
        violations.push(
            "concurrent replay did not return one first delivery and one durable replay".into(),
        );
    }
    if let Some(outcome) = &first_outcome {
        check_committed_state(
            port,
            &delivery.scope,
            outcome,
            "concurrent replay",
            violations,
        )
        .await;
    }
}

async fn check_fault_rollback<Port: PostgresInboxPort>(
    port: &Port,
    delivery: &InboxDelivery,
    fault: InboxFault,
    case: &str,
    violations: &mut Vec<String>,
) {
    if port.deliver(delivery.clone(), Some(fault)).await.is_ok() {
        violations.push(format!("{case} unexpectedly succeeded"));
    }
    match port.state(&delivery.scope).await {
        Ok(state) if state == InboxState::default() => {}
        Ok(_) => violations.push(format!("{case} left committed rows or effects")),
        Err(_) => violations.push(format!("could not inspect state after {case}")),
    }
    check_first_delivery(port, delivery, violations).await;
}

async fn check_committed_state<Port: PostgresInboxPort>(
    port: &Port,
    scope: &InboxScope,
    expected_outcome: &Value,
    case: &str,
    violations: &mut Vec<String>,
) {
    match port.state(scope).await {
        Ok(state)
            if state.inbox_rows == 1
                && state.domain_effects == 1
                && state.outbox_messages == 1
                && state.stored_outcome.as_ref() == Some(expected_outcome) => {}
        Ok(_) => violations.push(format!("{case} left the wrong committed state")),
        Err(_) => violations.push(format!("could not inspect state after {case}")),
    }
}

#[cfg(all(test, feature = "sqlx-postgres"))]
mod tests {
    use sqlx::Row as _;

    use super::*;

    #[derive(Clone)]
    struct SqlInboxPort {
        pool: sqlx::PgPool,
    }

    #[derive(Debug, thiserror::Error)]
    enum SqlInboxError {
        #[error("database operation failed")]
        Database(#[from] sqlx::Error),
        #[error("injected transaction failure")]
        Injected,
    }

    impl PostgresInboxPort for SqlInboxPort {
        type Error = SqlInboxError;

        fn deliver<'a>(
            &'a self,
            delivery: InboxDelivery,
            fault: Option<InboxFault>,
        ) -> InboxFuture<'a, InboxReceipt, Self::Error> {
            Box::pin(async move {
                let mut transaction = self.pool.begin().await?;
                let inserted = sqlx::query_scalar::<_, i32>(
                    "INSERT INTO conformance_inbox (owner_key, source, event_id, payload) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (owner_key, source, event_id) DO NOTHING \
                     RETURNING 1",
                )
                .bind(&delivery.scope.owner_key)
                .bind(&delivery.scope.source)
                .bind(&delivery.scope.event_id)
                .bind(&delivery.payload)
                .fetch_optional(&mut *transaction)
                .await?
                .is_some();
                if !inserted {
                    let outcome = sqlx::query_scalar::<_, Value>(
                        "SELECT outcome FROM conformance_inbox \
                         WHERE owner_key = $1 AND source = $2 AND event_id = $3",
                    )
                    .bind(&delivery.scope.owner_key)
                    .bind(&delivery.scope.source)
                    .bind(&delivery.scope.event_id)
                    .fetch_one(&mut *transaction)
                    .await?;
                    transaction.commit().await?;
                    return Ok(InboxReceipt {
                        disposition: InboxDisposition::Replay,
                        outcome,
                    });
                }
                if fault == Some(InboxFault::AfterInboxInsert) {
                    transaction.rollback().await?;
                    return Err(SqlInboxError::Injected);
                }
                if fault == Some(InboxFault::DomainWrite) {
                    transaction.rollback().await?;
                    return Err(SqlInboxError::Injected);
                }
                sqlx::query(
                    "INSERT INTO conformance_domain_effects (owner_key, source, event_id) \
                     VALUES ($1, $2, $3)",
                )
                .bind(&delivery.scope.owner_key)
                .bind(&delivery.scope.source)
                .bind(&delivery.scope.event_id)
                .execute(&mut *transaction)
                .await?;
                if fault == Some(InboxFault::OutboxWrite) {
                    transaction.rollback().await?;
                    return Err(SqlInboxError::Injected);
                }
                sqlx::query(
                    "INSERT INTO conformance_outbox (owner_key, source, event_id) \
                     VALUES ($1, $2, $3)",
                )
                .bind(&delivery.scope.owner_key)
                .bind(&delivery.scope.source)
                .bind(&delivery.scope.event_id)
                .execute(&mut *transaction)
                .await?;
                let outcome = serde_json::json!({
                    "status": "applied",
                    "reference": delivery.scope.event_id.clone(),
                });
                sqlx::query(
                    "UPDATE conformance_inbox SET outcome = $4 \
                     WHERE owner_key = $1 AND source = $2 AND event_id = $3",
                )
                .bind(&delivery.scope.owner_key)
                .bind(&delivery.scope.source)
                .bind(&delivery.scope.event_id)
                .bind(&outcome)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(InboxReceipt {
                    disposition: InboxDisposition::FirstDelivery,
                    outcome,
                })
            })
        }

        fn state<'a>(&'a self, scope: &'a InboxScope) -> InboxFuture<'a, InboxState, Self::Error> {
            Box::pin(async move {
                let row = sqlx::query(
                    "SELECT \
                         (SELECT count(*) FROM conformance_inbox \
                          WHERE owner_key = $1 AND source = $2 AND event_id = $3) AS inbox_rows, \
                         (SELECT count(*) FROM conformance_domain_effects \
                          WHERE owner_key = $1 AND source = $2 AND event_id = $3) AS domain_effects, \
                         (SELECT count(*) FROM conformance_outbox \
                          WHERE owner_key = $1 AND source = $2 AND event_id = $3) AS outbox_messages, \
                         (SELECT outcome FROM conformance_inbox \
                          WHERE owner_key = $1 AND source = $2 AND event_id = $3) AS stored_outcome",
                )
                .bind(&scope.owner_key)
                .bind(&scope.source)
                .bind(&scope.event_id)
                .fetch_one(&self.pool)
                .await?;
                Ok(InboxState {
                    inbox_rows: u64::try_from(row.try_get::<i64, _>("inbox_rows")?)
                        .unwrap_or(u64::MAX),
                    domain_effects: u64::try_from(row.try_get::<i64, _>("domain_effects")?)
                        .unwrap_or(u64::MAX),
                    outbox_messages: u64::try_from(row.try_get::<i64, _>("outbox_messages")?)
                        .unwrap_or(u64::MAX),
                    stored_outcome: row.try_get("stored_outcome")?,
                })
            })
        }

        fn discard_transient_state<'a>(&'a self) -> InboxFuture<'a, (), Self::Error> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon for PostgreSQL inbox concurrency coverage"]
    async fn postgres_inbox_conformance_includes_real_concurrent_replay()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = crate::start_postgres().await?;
        let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
        for statement in [
            "CREATE TABLE conformance_inbox (\
                 owner_key TEXT NOT NULL, source TEXT NOT NULL, event_id TEXT NOT NULL, \
                 payload JSONB NOT NULL, outcome JSONB, \
                 PRIMARY KEY (owner_key, source, event_id)\
             )",
            "CREATE TABLE conformance_domain_effects (\
                 owner_key TEXT NOT NULL, source TEXT NOT NULL, event_id TEXT NOT NULL, \
                 PRIMARY KEY (owner_key, source, event_id)\
             )",
            "CREATE TABLE conformance_outbox (\
                 owner_key TEXT NOT NULL, source TEXT NOT NULL, event_id TEXT NOT NULL, \
                 PRIMARY KEY (owner_key, source, event_id)\
             )",
        ] {
            sqlx::query(statement).execute(&pool).await?;
        }
        let port = SqlInboxPort { pool: pool.clone() };
        let cases = InboxConformanceCases::new(
            InboxDelivery::new(
                InboxScope::new("owner-a", "source-a", "event-1"),
                serde_json::json!({"kind": "example"}),
            ),
            "owner-b",
            "source-b",
        );

        check_postgres_inbox_conformance(&port, cases).await?;

        pool.close().await;
        drop(fixture);
        Ok(())
    }
}
