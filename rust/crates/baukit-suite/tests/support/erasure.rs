use super::*;
use baukit_erasure::{
    ErasureError, ErasureFuture, ErasureOutcome, ErasureService, ErasureState, IdentityRetention,
    PostgresErasureStore, ProductErasure,
};
use chrono::DateTime;
use sqlx::PgConnection;

const SUBJECT: &str = "stable-provider-subject";
const FIRST_KEY: &str = "suite-first-erasure-01";
const NEW_KEY: &str = "suite-new-erasure-key-01";

struct SubjectOwners(PgPool);

#[async_trait]
impl SuiteErasureOwnerLookup for SubjectOwners {
    async fn owner(&self, subject: &str) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar("SELECT id FROM users WHERE subject = $1")
            .bind(subject)
            .fetch_optional(&self.0)
            .await
    }

    async fn owner_in_transaction(
        &self,
        connection: &mut PgConnection,
        subject: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar("SELECT id FROM users WHERE subject = $1")
            .bind(subject)
            .fetch_optional(connection)
            .await
    }

    async fn lock_owner_in_transaction(
        &self,
        connection: &mut PgConnection,
        owner: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(owner)
            .execute(connection)
            .await?;
        Ok(())
    }
}

struct SubjectDeletion;

impl ProductErasure for SubjectDeletion {
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM users WHERE subject = $1")
                .bind(subject)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}

struct RecordingNotifier {
    inner: SuiteErasureNotificationService,
    subjects: Mutex<Vec<String>>,
}

#[async_trait]
impl SuiteErasureNotifier for RecordingNotifier {
    async fn notify_before_erasure(&self, subject: &str) {
        self.subjects
            .lock()
            .expect("notifier calls")
            .push(subject.to_owned());
        self.inner.notify_before_erasure(subject).await;
    }
}

struct Fixture {
    db: common::TestDatabase,
    peer: ScriptedWebhookReceiver,
    service: ErasureService,
    adapter: Arc<PostgresSuiteErasure>,
    links: SuiteLinkService,
    notifier: RecordingNotifier,
}

impl Fixture {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let mut db = common::postgres_database().await?;
        db.pool.close().await;
        db.pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(db._container.connection_url())
            .await?;
        for migration in [
            baukit_erasure::POSTGRES_MIGRATION_SQL,
            baukit_erasure::POSTGRES_REONBOARDING_MIGRATION_SQL,
            "ALTER TABLE users ADD COLUMN subject TEXT UNIQUE",
        ] {
            sqlx::raw_sql(migration).execute(&db.pool).await?;
        }
        let peer = ScriptedWebhookReceiver::start().await?;
        let port = Url::parse(&peer.url("/"))?.port().ok_or("peer port")?;
        let context = make_context(&db.pool, registry("alpha", "beta", 12340, port));
        let adapter = Arc::new(PostgresSuiteErasure::new(
            PostgresSuiteLinkStore::new(db.pool.clone()),
            Arc::new(SubjectOwners(db.pool.clone())),
            Arc::new(SubjectDeletion),
        ));
        let notifier = RecordingNotifier {
            inner: SuiteErasureNotificationService::new(
                adapter.clone(),
                Arc::new(SuiteDeliveryService::new(context.clone())),
            ),
            subjects: Mutex::new(Vec::new()),
        };
        let service = ErasureService::new(
            PostgresErasureStore::new(
                db.pool.clone(),
                baukit_config::Secret::new("suite-erasure-key-material-32-bytes".into()),
            )?,
            IdentityRetention::Retain,
        )?;
        Ok(Self {
            db,
            peer,
            service,
            adapter,
            links: SuiteLinkService::new(context),
            notifier,
        })
    }

    async fn onboard(
        &self,
        subject: &str,
        authenticated_at: Option<DateTime<Utc>>,
    ) -> Result<SuiteLink, Box<dyn Error>> {
        let owner = Uuid::now_v7();
        let mut transaction = self.db.pool.begin().await?;
        self.service
            .store()
            .guard_subject(&mut transaction, subject, authenticated_at)
            .await?;
        sqlx::query(
            "INSERT INTO users(id,email,username,display_name,subject) VALUES($1,$2,$3,$3,$4)",
        )
        .bind(owner)
        .bind(format!("{owner}@example.test"))
        .bind(owner.to_string())
        .bind(subject)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(exchange_link(&self.links, owner, None, None).await?.0)
    }

    async fn erase(
        &self,
        subject: &str,
        authenticated_at: Option<DateTime<Utc>>,
        key: &str,
    ) -> Result<ErasureOutcome, ErasureError> {
        erase_with_suite(
            &self.service,
            &self.notifier,
            subject,
            authenticated_at,
            key,
            self.adapter.as_ref(),
        )
        .await
    }

    async fn reonboard(
        &self,
    ) -> Result<(ErasureOutcome, SuiteLink, DateTime<Utc>), Box<dyn Error>> {
        let original = self.onboard(SUBJECT, None).await?;
        self.peer.push_response(ScriptedWebhookResponse::new(200));
        let receipt = self.erase(SUBJECT, None, FIRST_KEY).await?;
        assert_eq!(receipt.status, ErasureState::Completed);
        assert_eq!(
            *self.notifier.subjects.lock().expect("notifier calls"),
            [SUBJECT]
        );
        assert_eq!(self.peer.calls(), 1);
        let login: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&self.db.pool)
            .await?;
        let new_link = self.onboard(SUBJECT, Some(login)).await?;
        assert_ne!(original.user_id, new_link.user_id);
        assert_ne!(original.id, new_link.id);
        self.notifier
            .subjects
            .lock()
            .expect("notifier calls")
            .clear();
        Ok((receipt, new_link, login))
    }

    async fn assert_untouched(&self, link: &SuiteLink, expected_peer_calls: usize) -> TestResult {
        assert!(
            self.notifier
                .subjects
                .lock()
                .expect("notifier calls")
                .is_empty()
        );
        assert_eq!(self.peer.calls(), expected_peer_calls);
        let current = self.links.get(link.user_id, link.id).await?;
        assert_eq!(current.status, LinkStatus::Active);
        assert_eq!(current.revoked_at, None);
        assert_eq!(current.updated_at, link.updated_at);
        let owner: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
            .bind(link.user_id)
            .fetch_one(&self.db.pool)
            .await?;
        assert_eq!(owner, link.user_id);
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn old_key_replay_after_reonboarding_does_not_notify_or_revoke_new_links() -> TestResult {
    let fixture = Fixture::new().await?;
    let (receipt, link, login) = fixture.reonboard().await?;
    assert_eq!(
        fixture.erase(SUBJECT, Some(login), FIRST_KEY).await?,
        receipt
    );
    fixture.assert_untouched(&link, 1).await
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn fenced_login_after_reonboarding_does_not_notify_or_revoke_new_links() -> TestResult {
    let fixture = Fixture::new().await?;
    let (_, link, _) = fixture.reonboard().await?;
    let erased_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT erased_at FROM erasure_fences WHERE subject_hash = $1")
            .bind(fixture.service.store().subject_hash(SUBJECT))
            .fetch_one(&fixture.db.pool)
            .await?;
    let old_login = erased_at - chrono::Duration::seconds(1);
    assert!(matches!(
        fixture.erase(SUBJECT, Some(old_login), NEW_KEY).await,
        Err(ErasureError::ProfileErased)
    ));
    fixture.assert_untouched(&link, 1).await
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn invalid_key_does_not_notify_or_revoke_links() -> TestResult {
    let fixture = Fixture::new().await?;
    let link = fixture.onboard(SUBJECT, None).await?;
    assert!(matches!(
        fixture.erase(SUBJECT, None, "short").await,
        Err(ErasureError::InvalidKey)
    ));
    fixture.assert_untouched(&link, 0).await
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn foreign_key_does_not_notify_or_revoke_links() -> TestResult {
    let fixture = Fixture::new().await?;
    fixture
        .service
        .erase("other-subject", None, FIRST_KEY, fixture.adapter.as_ref())
        .await?;
    let link = fixture.onboard(SUBJECT, None).await?;
    assert!(matches!(
        fixture.erase(SUBJECT, None, FIRST_KEY).await,
        Err(ErasureError::Conflict)
    ));
    fixture.assert_untouched(&link, 0).await
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn first_erasure_notifies_once_and_deletes_the_current_profile_and_links() -> TestResult {
    let fixture = Fixture::new().await?;
    let link = fixture.onboard(SUBJECT, None).await?;
    fixture
        .peer
        .push_response(ScriptedWebhookResponse::new(200));
    let receipt = fixture.erase(SUBJECT, None, FIRST_KEY).await?;
    assert_eq!(receipt.status, ErasureState::Completed);
    assert_eq!(
        *fixture.notifier.subjects.lock().expect("notifier calls"),
        [SUBJECT]
    );
    assert_eq!(fixture.peer.calls(), 1);
    assert!(
        fixture.peer.received_requests()[0]
            .uri()
            .path()
            .ends_with(&format!("/suite/links/{}/revoke", link.remote_link_id))
    );
    let remaining: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM users WHERE subject=$1), (SELECT count(*) FROM suite_links WHERE user_id=$2)")
        .bind(SUBJECT).bind(link.user_id).fetch_one(&fixture.db.pool).await?;
    assert_eq!(remaining, (0, 0));
    Ok(())
}
