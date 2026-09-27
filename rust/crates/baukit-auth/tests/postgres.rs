use std::{collections::BTreeSet, error::Error, num::NonZeroU32, path::PathBuf, sync::Arc};

use baukit_auth::{
    ApiTokenError, ApiTokenPolicyRejection, ApiTokenService, ApiTokenStore, ApiTokenVerifier,
    IdentityVerifier, NewApiToken, PostgresApiTokenStore, Principal, VerificationError,
    erase_owner_api_tokens, purge_inactive_api_tokens,
};
use baukit_test::PostgresTestContainer;
use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use sqlx::PgPool;
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

const ACTIVE_LIMIT: NonZeroU32 = NonZeroU32::new(3).expect("three is not zero");
const CONCURRENT_ISSUES: usize = 10;
const CONCURRENT_TOUCHES: i64 = 32;
const PURGE_BATCH: NonZeroU32 = NonZeroU32::new(1).expect("one is not zero");

struct RejectingVerifier;

impl IdentityVerifier for RejectingVerifier {
    fn verify<'a>(
        &'a self,
        _token: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Principal, VerificationError>> + Send + 'a>,
    > {
        Box::pin(std::future::ready(Err(VerificationError::InvalidSignature)))
    }
}

fn instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0)
        .single()
        .expect("valid test instant")
}

async fn fixture() -> Result<(PostgresTestContainer, PgPool), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(
        "CREATE TABLE owners (id UUID PRIMARY KEY);
         ALTER TABLE api_tokens
             ADD CONSTRAINT api_tokens_owner_fk
             FOREIGN KEY (owner_id) REFERENCES owners (id) ON DELETE CASCADE;",
    )
    .execute(&pool)
    .await?;
    Ok((fixture, pool))
}

async fn owner(pool: &PgPool) -> Result<Uuid, TestError> {
    let owner_id = Uuid::now_v7();
    sqlx::query("INSERT INTO owners (id) VALUES ($1)")
        .bind(owner_id)
        .execute(pool)
        .await?;
    Ok(owner_id)
}

async fn token_count(pool: &PgPool) -> Result<i64, TestError> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM api_tokens")
        .fetch_one(pool)
        .await?)
}

fn service(pool: &PgPool) -> ApiTokenService {
    ApiTokenService::new(Arc::new(PostgresApiTokenStore::new(pool.clone())))
}

fn limit_rejection() -> Result<ApiTokenPolicyRejection, TestError> {
    Ok(ApiTokenPolicyRejection::new("api_tokens_active_limit")?
        .with_detail("maximum", ACTIVE_LIMIT.get())?)
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_token_and_its_grants_commit_together_or_not_at_all() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let tokens = service(&pool);
    let owner_id = owner(&pool).await?;
    let grants = BTreeSet::from(["records:read".to_owned(), "records:write".to_owned()]);

    let issued = tokens
        .issue_at(
            owner_id,
            NewApiToken::new("Deploy").with_grants(grants.clone()),
            instant(),
        )
        .await?;
    assert_eq!(issued.token.grants, grants);
    let listed = tokens.list_for_owner(owner_id).await?;
    assert_eq!(listed, vec![issued.token.clone()]);

    let verifier = ApiTokenVerifier::new(tokens.clone(), Arc::new(RejectingVerifier));
    let principal = verifier.verify(&issued.secret).await?;
    assert_eq!(principal.grants(), Some(&grants));
    assert_eq!(principal.subject(), owner_id.to_string());

    sqlx::query(
        "ALTER TABLE api_tokens ADD CONSTRAINT product_known_grants
         CHECK (grants <@ ARRAY['records:read', 'records:write']::TEXT[])",
    )
    .execute(&pool)
    .await?;
    let rejected = tokens
        .issue_at(
            owner_id,
            NewApiToken::new("Unknown grant").with_grants(["records:delete"]),
            instant(),
        )
        .await;
    assert!(matches!(rejected, Err(ApiTokenError::Storage(_))));

    let orphan = tokens
        .issue_at(
            Uuid::now_v7(),
            NewApiToken::new("No owner").with_grants(["records:read"]),
            instant(),
        )
        .await;
    assert!(matches!(orphan, Err(ApiTokenError::Storage(_))));
    assert_eq!(token_count(&pool).await?, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn the_active_token_limit_holds_under_concurrent_issue() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let rejection = limit_rejection()?;
    let store = PostgresApiTokenStore::new(pool.clone())
        .with_active_token_limit(ACTIVE_LIMIT, rejection.clone());
    let tokens = ApiTokenService::new(Arc::new(store));
    let owner_id = owner(&pool).await?;

    let expired = tokens
        .issue_at(
            owner_id,
            NewApiToken::new("Old").expiring_at(instant() + TimeDelta::minutes(1)),
            instant(),
        )
        .await?;
    let later = instant() + TimeDelta::hours(1);
    assert!(!expired.token.is_active_at(later));

    let attempts = (0..CONCURRENT_ISSUES).map(|index| {
        let tokens = tokens.clone();
        tokio::spawn(async move {
            tokens
                .issue_at(owner_id, NewApiToken::new(format!("Token {index}")), later)
                .await
        })
    });
    let mut issued = 0;
    for attempt in attempts.collect::<Vec<_>>() {
        match attempt.await? {
            Ok(_) => issued += 1,
            Err(ApiTokenError::PolicyRejected(actual)) => assert_eq!(actual, rejection),
            Err(error) => return Err(error.into()),
        }
    }

    assert_eq!(issued, ACTIVE_LIMIT.get());
    assert_eq!(token_count(&pool).await?, i64::from(ACTIVE_LIMIT.get()) + 1);

    let other_owner = owner(&pool).await?;
    tokens
        .issue_at(other_owner, NewApiToken::new("Separate owner"), later)
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_revoked_token_stops_authenticating_and_only_its_owner_can_revoke_it()
-> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let tokens = service(&pool);
    let owner_id = owner(&pool).await?;
    let stranger = owner(&pool).await?;
    let issued = tokens
        .issue_at(owner_id, NewApiToken::new("CLI"), instant())
        .await?;

    let foreign = tokens.revoke_at(stranger, issued.token.id, instant()).await;
    assert!(matches!(foreign, Err(ApiTokenError::NotFound)));
    tokens.verify_at(&issued.secret, instant()).await?;

    let revoked_at = instant() + TimeDelta::minutes(5);
    tokens
        .revoke_at(owner_id, issued.token.id, revoked_at)
        .await?;
    let verified = tokens.verify_at(&issued.secret, revoked_at).await;
    assert!(matches!(verified, Err(ApiTokenError::Invalid)));
    let again = tokens
        .revoke_at(owner_id, issued.token.id, revoked_at)
        .await;
    assert!(matches!(again, Err(ApiTokenError::NotFound)));

    let listed = tokens.list_for_owner(owner_id).await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].revoked_at, Some(revoked_at));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn an_expired_token_is_reported_and_its_last_use_is_left_alone() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let tokens = service(&pool);
    let owner_id = owner(&pool).await?;
    let expires_at = instant() + TimeDelta::hours(1);
    let issued = tokens
        .issue_at(
            owner_id,
            NewApiToken::new("Short").expiring_at(expires_at),
            instant(),
        )
        .await?;

    let used_at = instant() + TimeDelta::minutes(30);
    tokens.verify_at(&issued.secret, used_at).await?;
    let expired = tokens.verify_at(&issued.secret, expires_at).await;
    assert!(matches!(expired, Err(ApiTokenError::Expired)));

    let listed = tokens.list_for_owner(owner_id).await?;
    assert_eq!(listed[0].last_used_at, Some(used_at));
    assert_eq!(listed[0].expires_at, Some(expires_at));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn erasing_an_owner_erases_only_their_tokens() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let tokens = service(&pool);
    let cascaded = owner(&pool).await?;
    let erased = owner(&pool).await?;
    let kept = owner(&pool).await?;
    for owner_id in [cascaded, erased, erased, kept] {
        tokens
            .issue_at(owner_id, NewApiToken::new("Token"), instant())
            .await?;
    }

    sqlx::query("DELETE FROM owners WHERE id = $1")
        .bind(cascaded)
        .execute(&pool)
        .await?;
    assert!(tokens.list_for_owner(cascaded).await?.is_empty());

    let mut transaction = pool.begin().await?;
    assert_eq!(erase_owner_api_tokens(&mut *transaction, erased).await?, 2);
    transaction.commit().await?;
    assert!(tokens.list_for_owner(erased).await?.is_empty());
    assert_eq!(tokens.list_for_owner(kept).await?.len(), 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_last_used_updates_keep_the_latest_instant() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let store = Arc::new(PostgresApiTokenStore::new(pool.clone()));
    let tokens = ApiTokenService::new(store.clone());
    let owner_id = owner(&pool).await?;
    let issued = tokens
        .issue_at(owner_id, NewApiToken::new("Busy"), instant())
        .await?;
    let token_id = issued.token.id;

    let offsets = (0..CONCURRENT_TOUCHES).map(|offset| (offset * 7) % CONCURRENT_TOUCHES);
    let touches = offsets
        .map(|offset| {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .touch_last_used(token_id, instant() + TimeDelta::seconds(offset))
                    .await
            })
        })
        .collect::<Vec<_>>();
    for touch in touches {
        touch.await??;
    }
    let latest = instant() + TimeDelta::seconds(CONCURRENT_TOUCHES - 1);
    assert_eq!(
        tokens.list_for_owner(owner_id).await?[0].last_used_at,
        Some(latest)
    );

    store.touch_last_used(token_id, instant()).await?;
    assert_eq!(
        tokens.list_for_owner(owner_id).await?[0].last_used_at,
        Some(latest)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn purge_removes_only_tokens_inactive_before_the_cutoff() -> Result<(), TestError> {
    let (_container, pool) = fixture().await?;
    let tokens = service(&pool);
    let owner_id = owner(&pool).await?;
    let issue = |name: &'static str, expires_at: Option<DateTime<Utc>>| {
        let tokens = tokens.clone();
        async move {
            let mut request = NewApiToken::new(name);
            request.expires_at = expires_at;
            tokens.issue_at(owner_id, request, instant()).await
        }
    };
    let cutoff = instant() + TimeDelta::days(30);
    let active = issue("Active", None).await?;
    let expired_long_ago = issue("Expired", Some(instant() + TimeDelta::days(1))).await?;
    let expires_after_cutoff = issue("Expiring", Some(cutoff + TimeDelta::days(1))).await?;
    let revoked_long_ago = issue("Revoked", None).await?;
    let revoked_recently = issue("Recently revoked", None).await?;
    tokens
        .revoke_at(owner_id, revoked_long_ago.token.id, instant())
        .await?;
    tokens
        .revoke_at(owner_id, revoked_recently.token.id, cutoff)
        .await?;

    let mut purged = 0;
    loop {
        let deleted = purge_inactive_api_tokens(&pool, cutoff, PURGE_BATCH).await?;
        purged += deleted;
        if deleted < u64::from(PURGE_BATCH.get()) {
            break;
        }
    }

    assert_eq!(purged, 2);
    let remaining = tokens
        .list_for_owner(owner_id)
        .await?
        .into_iter()
        .map(|token| token.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        remaining,
        BTreeSet::from([
            active.token.id,
            expires_after_cutoff.token.id,
            revoked_recently.token.id
        ])
    );
    assert!(!remaining.contains(&expired_long_ago.token.id));
    Ok(())
}
