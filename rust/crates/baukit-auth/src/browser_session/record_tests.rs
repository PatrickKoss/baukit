use std::error::Error;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_credential_vault::{CredentialCipher, CredentialSecrets};
use jiff::Timestamp;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    POSTGRES_BROWSER_SESSION_RECORD_IDS_MIGRATION_SQL, POSTGRES_BROWSER_SESSIONS_MIGRATION_SQL,
    crypto::{self, ID_TOKEN_FIELD, REFRESH_TOKEN_FIELD, aad_scope},
    store::{PostgresSessionStore, SessionRow, SessionStore as _},
};

type TestError = Box<dyn Error + Send + Sync>;
type TestResult = Result<(), TestError>;

const SUBJECT: &str = "alice";
const SECRET_HASH_LENGTH: usize = 32;

fn cipher() -> Result<CredentialCipher, TestError> {
    Ok(CredentialCipher::parse(&format!(
        "1:{}",
        STANDARD.encode([9_u8; 32])
    ))?)
}

fn row(id: Uuid, key_version: i32, record_id: Option<Uuid>) -> SessionRow {
    let now = Timestamp::now();
    SessionRow {
        id,
        secret_hash: vec![0; SECRET_HASH_LENGTH],
        family_id: Uuid::now_v7(),
        subject: SUBJECT.to_owned(),
        key_version,
        record_id,
        refresh_token: Vec::new(),
        id_token: None,
        created_at: now,
        last_used_at: now,
        idle_expires_at: now,
        absolute_expires_at: now,
        revoked_at: None,
        rotated_to: None,
        grace_access_token: None,
        grace_expires_at: None,
        grace_until: None,
    }
}

fn sealed_row(cipher: &CredentialCipher, refresh: &str) -> Result<SessionRow, TestError> {
    let id = Uuid::now_v7();
    let mut sealed = crypto::seal(
        cipher,
        id,
        SUBJECT,
        &[(REFRESH_TOKEN_FIELD, refresh), (ID_TOKEN_FIELD, "id-token")],
    )?;
    let mut row = row(id, sealed.key_version, Some(sealed.record_id));
    row.refresh_token = sealed
        .fields
        .remove(REFRESH_TOKEN_FIELD)
        .ok_or("no refresh token")?;
    row.id_token = sealed.fields.remove(ID_TOKEN_FIELD);
    Ok(row)
}

fn v1_packed(cipher: &CredentialCipher, id: Uuid, value: &str) -> Result<Vec<u8>, TestError> {
    let secrets = CredentialSecrets::new().with(REFRESH_TOKEN_FIELD, value.as_bytes().to_vec())?;
    let encrypted = cipher.encrypt(aad_scope(id, SUBJECT), &secrets)?;
    let field = encrypted
        .fields
        .get(REFRESH_TOKEN_FIELD)
        .ok_or("no field")?;
    let mut packed = field.nonce.clone();
    packed.extend_from_slice(&field.ciphertext);
    Ok(packed)
}

async fn insert(store: &PostgresSessionStore, row: &SessionRow) -> TestResult {
    let mut transaction = store.begin().await?;
    transaction.insert(row).await?;
    transaction.commit().await?;
    Ok(())
}

async fn load(store: &PostgresSessionStore, id: Uuid) -> Result<SessionRow, TestError> {
    let mut transaction = store.begin().await?;
    Ok(transaction.lock(id).await?.ok_or("row missing")?)
}

async fn opens(store: &PostgresSessionStore, cipher: &CredentialCipher, id: Uuid) -> bool {
    match load(store, id).await {
        Ok(row) => crypto::open(cipher, &row, REFRESH_TOKEN_FIELD).is_ok(),
        Err(_) => false,
    }
}

async fn execute(pool: &PgPool, sql: &'static str, first: Uuid, second: Uuid) -> TestResult {
    sqlx::query(sql)
        .bind(first)
        .bind(second)
        .execute(pool)
        .await?;
    Ok(())
}

const TAMPERING: [&str; 4] = [
    "UPDATE baukit_auth_browser_sessions SET id_token = NULL WHERE id = $1 AND $2 = $2",
    "UPDATE baukit_auth_browser_sessions SET record_id = NULL WHERE id = $1 AND $2 = $2",
    "UPDATE baukit_auth_browser_sessions t SET id_token = d.id_token
     FROM baukit_auth_browser_sessions d WHERE t.id = $1 AND d.id = $2",
    "UPDATE baukit_auth_browser_sessions t SET record_id = d.record_id
     FROM baukit_auth_browser_sessions d WHERE t.id = $1 AND d.id = $2",
];

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn v2_rows_reject_tampering_and_v1_rows_written_before_the_migration_still_open() -> TestResult
{
    let postgres = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(postgres.connection_url()).await?;
    sqlx::raw_sql(POSTGRES_BROWSER_SESSIONS_MIGRATION_SQL)
        .execute(&pool)
        .await?;
    let cipher = cipher()?;
    let legacy = Uuid::now_v7();
    let mut old = row(legacy, 1, None);
    old.refresh_token = v1_packed(&cipher, legacy, "legacy-refresh")?;
    sqlx::query(
        "INSERT INTO baukit_auth_browser_sessions (id, secret_hash, family_id, subject,
         key_version, refresh_token, created_at, last_used_at, idle_expires_at,
         absolute_expires_at) VALUES ($1, $2, $3, $4, $5, $6, now(), now(), now(), now())",
    )
    .bind(old.id)
    .bind(&old.secret_hash)
    .bind(old.family_id)
    .bind(&old.subject)
    .bind(old.key_version)
    .bind(&old.refresh_token)
    .execute(&pool)
    .await?;
    sqlx::raw_sql(POSTGRES_BROWSER_SESSION_RECORD_IDS_MIGRATION_SQL)
        .execute(&pool)
        .await?;
    let store = PostgresSessionStore::new(pool.clone());
    let opened = crypto::open(&cipher, &load(&store, legacy).await?, REFRESH_TOKEN_FIELD)?;
    assert_eq!(opened.as_str(), "legacy-refresh");

    let donor = sealed_row(&cipher, "donor")?;
    insert(&store, &donor).await?;
    for sql in TAMPERING {
        let target = sealed_row(&cipher, "target")?;
        insert(&store, &target).await?;
        assert!(opens(&store, &cipher, target.id).await, "{sql}");
        execute(&pool, sql, target.id, donor.id).await?;
        assert!(!opens(&store, &cipher, target.id).await, "{sql}");
    }
    assert!(opens(&store, &cipher, donor.id).await);
    Ok(())
}
