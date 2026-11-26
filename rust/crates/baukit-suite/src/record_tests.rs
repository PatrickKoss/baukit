use std::error::Error;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_credential_vault::{CredentialCipher, CredentialSecrets};
use jiff::Timestamp;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    SUITE_LOCK_ORDER_MIGRATION_SQL, SUITE_MIGRATION_SQL, SUITE_RECORD_IDS_MIGRATION_SQL,
    SUITE_RUNTIME_MIGRATION_SQL,
    adapters::postgres::PostgresSuiteLinkStore,
    domain::{DeliveryHealth, EncryptedPayload, LinkRole, LinkStatus, RewardMode, SuiteLink},
    ports::SuiteLinkStore as _,
    secret_cipher::SecretCipher,
};

type TestError = Box<dyn Error + Send + Sync>;
type TestResult = Result<(), TestError>;

const FIELD: &str = "link_secret";

fn cipher() -> Result<CredentialCipher, TestError> {
    Ok(CredentialCipher::parse(&format!(
        "1:{}",
        STANDARD.encode([7_u8; 32])
    ))?)
}

fn link(owner: Uuid, peer: &str) -> SuiteLink {
    let now = Timestamp::now();
    SuiteLink {
        id: Uuid::now_v7(),
        user_id: owner,
        peer_app: peer.into(),
        role: LinkRole::Initiator,
        remote_link_id: Uuid::now_v7(),
        remote_subject: "remote".into(),
        remote_display_name: None,
        suite_subject: None,
        status: LinkStatus::Active,
        secret: EncryptedPayload {
            ciphertext: Vec::new(),
            nonce: Vec::new(),
            key_version: 1,
            record_id: None,
        },
        sends: Vec::new(),
        receives: Vec::new(),
        share_xp: false,
        reward_mode: RewardMode::Native,
        delivery_health: DeliveryHealth::Healthy,
        consecutive_failures: 0,
        last_delivery_at: None,
        last_failure_at: None,
        last_failure_code: None,
        last_received_at: None,
        created_at: now,
        updated_at: now,
        revoked_at: None,
    }
}

async fn migrated_before_record_ids(pool: &PgPool) -> TestResult {
    for migration in [
        baukit_jobs::POSTGRES_MIGRATION_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0002_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0003_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0004_SQL,
        SUITE_MIGRATION_SQL,
        SUITE_RUNTIME_MIGRATION_SQL,
        SUITE_LOCK_ORDER_MIGRATION_SQL,
    ] {
        sqlx::raw_sql(migration).execute(pool).await?;
    }
    Ok(())
}

async fn insert_v1_link(pool: &PgPool, owner: Uuid, plaintext: &[u8]) -> Result<Uuid, TestError> {
    let id = Uuid::now_v7();
    let secrets = CredentialSecrets::new().with(FIELD, plaintext.to_vec())?;
    let encrypted = cipher()?.encrypt(id, &secrets)?;
    let field = encrypted.fields.get(FIELD).ok_or("no field")?;
    sqlx::query(
        "INSERT INTO suite_links(id,user_id,peer_app,role,remote_link_id,remote_subject,
         secret_ciphertext,secret_nonce,secret_key_version,sends,receives)
         VALUES($1,$2,'legacy','authorizer',$3,'remote',$4,$5,$6,'{}','{}')",
    )
    .bind(id)
    .bind(owner)
    .bind(Uuid::now_v7())
    .bind(&field.ciphertext)
    .bind(&field.nonce)
    .bind(encrypted.key_version)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn insert_v2_link(
    store: &PostgresSuiteLinkStore,
    secrets: &SecretCipher,
    owner: Uuid,
    peer: &str,
) -> Result<Uuid, TestError> {
    let mut link = link(owner, peer);
    link.secret = secrets.encrypt(link.id, FIELD, peer.as_bytes())?;
    let mut tx = store.begin_transaction().await?;
    store.insert_link_in_transaction(&mut tx, &link).await?;
    store.commit_transaction(tx).await?;
    Ok(link.id)
}

async fn opened(
    store: &PostgresSuiteLinkStore,
    secrets: &SecretCipher,
    id: Uuid,
) -> Result<Option<Vec<u8>>, TestError> {
    let link = store.find_link(id).await?.ok_or("link missing")?;
    Ok(secrets.decrypt(link.id, FIELD, &link.secret).ok())
}

const TAMPERING: [&str; 4] = [
    "UPDATE suite_links SET secret_ciphertext='\\x'::bytea WHERE id=$1 AND $2 = $2",
    "UPDATE suite_links SET secret_record_id=NULL WHERE id=$1 AND $2 = $2",
    "UPDATE suite_links t SET secret_ciphertext=d.secret_ciphertext, secret_nonce=d.secret_nonce
     FROM suite_links d WHERE t.id=$1 AND d.id=$2",
    "UPDATE suite_links t SET secret_record_id=d.secret_record_id
     FROM suite_links d WHERE t.id=$1 AND d.id=$2",
];

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn v2_secrets_reject_tampering_and_v1_rows_written_before_the_migration_still_open()
-> TestResult {
    let postgres = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(postgres.connection_url()).await?;
    migrated_before_record_ids(&pool).await?;
    let owner = Uuid::now_v7();
    let legacy = insert_v1_link(&pool, owner, b"legacy-secret").await?;
    sqlx::raw_sql(SUITE_RECORD_IDS_MIGRATION_SQL)
        .execute(&pool)
        .await?;
    let store = PostgresSuiteLinkStore::new(pool.clone());
    let secrets = SecretCipher::new(Some(cipher()?));
    assert_eq!(
        opened(&store, &secrets, legacy).await?.as_deref(),
        Some(b"legacy-secret".as_slice())
    );

    let donor = insert_v2_link(&store, &secrets, owner, "donor").await?;
    for (index, sql) in TAMPERING.into_iter().enumerate() {
        let peer = format!("target-{index}");
        let target = insert_v2_link(&store, &secrets, owner, &peer).await?;
        assert_eq!(
            opened(&store, &secrets, target).await?.as_deref(),
            Some(peer.as_bytes())
        );
        sqlx::query(sql)
            .bind(target)
            .bind(donor)
            .execute(&pool)
            .await?;
        assert_eq!(opened(&store, &secrets, target).await?, None, "{sql}");
    }
    assert_eq!(
        opened(&store, &secrets, donor).await?.as_deref(),
        Some(b"donor".as_slice())
    );
    Ok(())
}
