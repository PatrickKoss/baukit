use std::sync::Arc;

use baukit_integrations::oauth_state::{
    OAuthStateError, OAuthStateService, Pkce, StartOAuthFlow,
    postgres::{OAUTH_STATE_MIGRATION_SQL, PostgresOAuthStateStore},
};
use sqlx::{PgPool, postgres::PgPoolOptions};

const RACERS: usize = 8;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn pool(url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await?;
    sqlx::raw_sql(OAUTH_STATE_MIGRATION_SQL)
        .execute(&pool)
        .await?;
    Ok(pool)
}

fn input() -> StartOAuthFlow {
    StartOAuthFlow {
        user: "user-1".into(),
        provider: "google".into(),
        redirect: "/linked".into(),
        pkce: Pkce::S256,
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn concurrent_completes_have_exactly_one_winner() -> TestResult {
    let container = baukit_test::start_postgres().await?;
    let pool = pool(container.connection_url()).await?;
    let service = OAuthStateService::new(Arc::new(PostgresOAuthStateStore::new(pool.clone())));
    let started = service.start(input()).await?;

    let mut tasks = Vec::new();
    for _ in 0..RACERS {
        let service = service.clone();
        let state = started.state.clone();
        let binding = started.binding.clone();
        tasks.push(tokio::spawn(async move {
            service.complete(&state, &binding).await
        }));
    }
    let mut winners = 0;
    for task in tasks {
        match task.await? {
            Ok(done) => {
                assert_eq!(done.user, "user-1");
                assert!(done.pkce_verifier.is_some());
                winners += 1;
            }
            Err(OAuthStateError::AlreadyUsed) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    assert_eq!(winners, 1);

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_states")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn postgres_store_reports_each_failure() -> TestResult {
    let container = baukit_test::start_postgres().await?;
    let pool = pool(container.connection_url()).await?;
    let service = OAuthStateService::new(Arc::new(PostgresOAuthStateStore::new(pool)));
    let now = chrono::Utc::now();
    let first = service.start_at(input(), now).await?;
    let second = service.start_at(input(), now).await?;

    let mismatch = service
        .complete_at(&first.state, &second.binding, now)
        .await
        .expect_err("mismatch");
    assert!(matches!(mismatch, OAuthStateError::BrowserMismatch));
    let used = service
        .complete_at(&first.state, &first.binding, now)
        .await
        .expect_err("used");
    assert!(matches!(used, OAuthStateError::AlreadyUsed));
    let expired = service
        .complete_at(
            &second.state,
            &second.binding,
            now + chrono::Duration::minutes(11),
        )
        .await
        .expect_err("expired");
    assert!(matches!(expired, OAuthStateError::Expired));
    let third = service.start_at(input(), now).await?;
    let done = service
        .complete_at(&third.state, &third.binding, now)
        .await?;
    assert_eq!(done.provider, "google");
    Ok(())
}
