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
    services::reseal::reseal_legacy_secrets,
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

fn v1_field(id: Uuid, field: &str, value: &[u8]) -> Result<(Vec<u8>, Vec<u8>), TestError> {
    let secrets = CredentialSecrets::new().with(field, value.to_vec())?;
    let mut encrypted = cipher()?.encrypt(id, &secrets)?;
    let field = encrypted.fields.remove(field).ok_or("no field")?;
    Ok((field.ciphertext, field.nonce))
}

async fn insert_v1_request(pool: &PgPool, owner: Uuid) -> Result<Uuid, TestError> {
    let id = Uuid::now_v7();
    let (verifier_ciphertext, verifier_nonce) = v1_field(id, "pkce_verifier", b"verifier")?;
    let (secret_ciphertext, secret_nonce) = v1_field(id, FIELD, b"exchange")?;
    sqlx::query(
        "INSERT INTO suite_link_requests(id,user_id,peer_app,state_hash,verifier_ciphertext,
         verifier_nonce,verifier_key_version,client_state_nonce,return_url,expires_at,created_at,
         initiator_link_id,secret_ciphertext,secret_nonce,secret_key_version)
         VALUES($1,$2,'beta',$3,$4,$5,1,'nonce','https://example.test/',now()+interval '10 minutes',
         now(),$6,$7,$8,1)",
    )
    .bind(id)
    .bind(owner)
    .bind(vec![3_u8; 32])
    .bind(verifier_ciphertext)
    .bind(verifier_nonce)
    .bind(Uuid::now_v7())
    .bind(secret_ciphertext)
    .bind(secret_nonce)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn request_payloads(pool: &PgPool, id: Uuid) -> Result<[EncryptedPayload; 2], TestError> {
    type Row = (
        Vec<u8>,
        Vec<u8>,
        i32,
        Option<Uuid>,
        Vec<u8>,
        Vec<u8>,
        i32,
        Option<Uuid>,
    );
    let row: Row = sqlx::query_as(
        "SELECT verifier_ciphertext, verifier_nonce, verifier_key_version, verifier_record_id,
         secret_ciphertext, secret_nonce, secret_key_version, secret_record_id
         FROM suite_link_requests WHERE id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok([
        EncryptedPayload {
            ciphertext: row.0,
            nonce: row.1,
            key_version: row.2,
            record_id: row.3,
        },
        EncryptedPayload {
            ciphertext: row.4,
            nonce: row.5,
            key_version: row.6,
            record_id: row.7,
        },
    ])
}

async fn link_secret(
    store: &PostgresSuiteLinkStore,
    id: Uuid,
) -> Result<EncryptedPayload, TestError> {
    Ok(store.find_link(id).await?.ok_or("link missing")?.secret)
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn reseal_converts_v1_secrets_once_skips_unreadable_ones_and_leaves_v2_alone() -> TestResult {
    let postgres = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(postgres.connection_url()).await?;
    migrated_before_record_ids(&pool).await?;
    let owner = Uuid::now_v7();
    let legacy = [
        insert_v1_link(&pool, Uuid::now_v7(), b"first").await?,
        insert_v1_link(&pool, Uuid::now_v7(), b"second").await?,
        insert_v1_link(&pool, Uuid::now_v7(), b"third").await?,
    ];
    let unreadable = insert_v1_link(&pool, Uuid::now_v7(), b"broken").await?;
    sqlx::query("UPDATE suite_links SET secret_nonce=$2 WHERE id=$1")
        .bind(unreadable)
        .bind(vec![0_u8; 12])
        .execute(&pool)
        .await?;
    let request = insert_v1_request(&pool, owner).await?;
    sqlx::raw_sql(SUITE_RECORD_IDS_MIGRATION_SQL)
        .execute(&pool)
        .await?;
    let store = PostgresSuiteLinkStore::new(pool.clone());
    let secrets = SecretCipher::new(Some(cipher()?));
    let current = insert_v2_link(&store, &secrets, owner, "current").await?;
    let current_before = link_secret(&store, current).await?;

    let first = reseal_legacy_secrets(&store, &secrets, 2).await?;
    assert_eq!((first.resealed_secrets, first.unreadable_secrets), (5, 1));
    for (id, plaintext) in legacy.into_iter().zip(["first", "second", "third"]) {
        let secret = link_secret(&store, id).await?;
        assert!(secret.record_id.is_some());
        assert_eq!(
            secrets.decrypt(id, FIELD, &secret)?,
            plaintext.as_bytes().to_vec()
        );
    }
    let [verifier, exchange] = request_payloads(&pool, request).await?;
    assert!(verifier.record_id.is_some() && exchange.record_id.is_some());
    assert_eq!(
        secrets.decrypt(request, "pkce_verifier", &verifier)?,
        b"verifier".to_vec()
    );
    assert_eq!(
        secrets.decrypt(request, FIELD, &exchange)?,
        b"exchange".to_vec()
    );
    assert_eq!(link_secret(&store, unreadable).await?.record_id, None);
    assert_eq!(link_secret(&store, current).await?, current_before);

    let second = reseal_legacy_secrets(&store, &secrets, 2).await?;
    assert_eq!((second.resealed_secrets, second.unreadable_secrets), (0, 1));
    assert_eq!(link_secret(&store, current).await?, current_before);
    Ok(())
}
