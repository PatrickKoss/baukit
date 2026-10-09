use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use baukit_core::webhook_signature::{sign_webhook_hmac_sha256, verify_webhook_hmac_sha256};
use baukit_credential_vault::{CredentialCipher, EncryptedCredentials, EncryptedField};
use baukit_events::EventEnvelope;
use baukit_jobs::{JobStore, NewJob, PostgresJobStore};
use baukit_test::{ScriptedWebhookReceiver, ScriptedWebhookResponse};
use chrono::{SubsecRound as _, Utc};

use baukit_ratelimit::InMemoryRateLimitStore;
use baukit_suite::adapters::peer::ReqwestSuitePeerClient;
use baukit_suite::adapters::postgres::*;
use baukit_suite::domain::*;
use baukit_suite::ports::*;
use baukit_suite::services::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    error::Error,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;
#[derive(Default)]
struct Applier {
    calls: Mutex<Vec<(Uuid, RewardMode, bool)>>,
    invalid: bool,
}
#[async_trait]
impl SuiteEventApplier for Applier {
    async fn apply(
        &self,
        tx: &mut sqlx::PgConnection,
        event: SuiteApplyEvent<'_>,
    ) -> Result<AppliedOutcome, SuiteStoreError> {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM suite_inbound_events WHERE link_id=$1 AND event_id=$2)",
        )
        .bind(event.link.id)
        .bind(&event.envelope.event_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| SuiteStoreError::Storage(e.to_string()))?;
        assert!(present);
        sqlx::query("INSERT INTO applied_events(owner_id,event_id,replay,reward_mode,payload) VALUES($1,$2,$3,$4,$5)")
            .bind(event.owner_id).bind(&event.envelope.event_id).bind(event.replay).bind(event.reward_mode.as_str()).bind(serde_json::Value::Object(event.payload.as_map().clone()))
            .execute(&mut *tx).await.map_err(|error| SuiteStoreError::Storage(error.to_string()))?;
        if self.invalid {
            return Err(SuiteStoreError::InvalidData(
                "corrupt applier data".to_owned(),
            ));
        }
        self.calls.lock().expect("valid test value").push((
            event.owner_id,
            event.link.reward_mode,
            event.replay,
        ));
        Ok(AppliedOutcome {
            outcome: if event.replay {
                AppliedOutcomeStatus::NoRule
            } else {
                AppliedOutcomeStatus::Granted
            },
            ledger_entry_id: (!event.replay).then(|| "recorded-ledger".to_owned()),
        })
    }
}
fn cipher() -> CredentialCipher {
    CredentialCipher::parse(&format!("1:{}", STANDARD.encode([7u8; 32]))).expect("valid test value")
}

fn registry(own: &str, peer: &str, own_port: u16, peer_port: u16) -> Arc<PeerRegistry> {
    Arc::new(
        PeerRegistry::new(
            own,
            PEERS,
            PeerRegistrySettings {
                public_api_url: Some(format!("http://127.0.0.1:{own_port}/api/v1")),
                public_web_url: Some(format!("http://127.0.0.1:{own_port}")),
                peers: BTreeMap::from([(
                    peer.to_owned(),
                    PeerUrls {
                        api_url: Some(format!("http://127.0.0.1:{peer_port}/api/v1")),
                        web_url: Some(format!("http://127.0.0.1:{peer_port}")),
                    },
                )]),
                allow_loopback: true,
            },
        )
        .expect("valid test value"),
    )
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Result<(StatusCode, Value), Box<dyn Error>> {
    let mut r = Request::builder()
        .method(method)
        .uri(format!("/api/v1{path}"))
        .header("content-type", "application/json");
    if let Some(token) = token {
        r = r.header("authorization", format!("Bearer {token}"))
    }
    let response = app
        .clone()
        .oneshot(r.body(Body::from(serde_json::to_vec(&body)?))?)
        .await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?;
    Ok((
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        },
    ))
}

async fn user(pool: &PgPool, name: &str) -> Result<Uuid, Box<dyn Error>> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO users(id,email,username,display_name) VALUES($1,$2,$3,$3)")
        .bind(id)
        .bind(format!("{name}@example.test"))
        .bind(name)
        .execute(pool)
        .await?;
    Ok(id)
}

fn query(url: &str, key: &str) -> String {
    Url::parse(url)
        .expect("valid test value")
        .query_pairs()
        .find(|(k, _)| k == key)
        .expect("valid test value")
        .1
        .into_owned()
}

fn secret(link: &SuiteLink) -> Vec<u8> {
    cipher()
        .decrypt(&EncryptedCredentials {
            scope_id: link.id,
            key_version: link.secret.key_version,
            fields: BTreeMap::from([(
                "link_secret".to_owned(),
                EncryptedField {
                    ciphertext: link.secret.ciphertext.clone(),
                    nonce: link.secret.nonce.clone(),
                },
            )]),
        })
        .expect("valid test value")
        .get("link_secret")
        .expect("valid test value")
        .to_vec()
}

fn registry_with_hebkit() -> Result<Arc<PeerRegistry>, Box<dyn Error>> {
    let mut peers: Value = serde_json::from_str(PEERS)?;
    let mut hebkit = peers["peers"][1].clone();
    hebkit["id"] = json!("hebkit");
    hebkit["scheme"] = json!("hebkit");
    hebkit["displayName"] = json!("Hebkit");
    peers["peers"].as_array_mut().expect("peers").push(hebkit);
    let urls = |port| PeerUrls {
        api_url: Some(format!("http://127.0.0.1:{port}/api/v1")),
        web_url: Some(format!("http://127.0.0.1:{port}")),
    };
    Ok(Arc::new(PeerRegistry::new(
        "alpha",
        &peers.to_string(),
        PeerRegistrySettings {
            public_api_url: Some("http://127.0.0.1:12340/api/v1".into()),
            public_web_url: Some("http://127.0.0.1:12340".into()),
            peers: BTreeMap::from([("beta".into(), urls(12341)), ("hebkit".into(), urls(12342))]),
            allow_loopback: true,
        },
    )?))
}

async fn authorizer_fixture()
-> Result<(common::TestDatabase, Arc<SuiteContext>, Uuid), Box<dyn Error>> {
    let db = common::postgres_database().await?;
    let owner = user(&db.pool, "suite-owner").await?;
    let context = make_context(&db.pool, registry_with_hebkit()?);
    Ok((db, context, owner))
}

async fn exchange_link(
    service: &SuiteLinkService,
    owner: Uuid,
    suite_subject: Option<String>,
    peer_subject: Option<String>,
) -> Result<(SuiteLink, ExchangeRequest), Box<dyn Error>> {
    let verifier = URL_SAFE_NO_PAD.encode([9u8; 32]);
    let authorization = service
        .authorize(
            SuiteUser {
                id: owner,
                suite_subject,
                display_name: None,
            },
            AuthorizationRequest {
                client: "beta".to_owned(),
                state: verifier.clone(),
                code_challenge: pkce_challenge(&verifier)?,
                hint: None,
            },
            Utc::now(),
        )
        .await?;
    let request = ExchangeRequest {
        client: "beta".to_owned(),
        code: query(&authorization.redirect_url, "code"),
        code_verifier: verifier.clone(),
        initiator_link_id: Uuid::now_v7(),
        link_secret: URL_SAFE_NO_PAD.encode(Uuid::now_v7().as_bytes().repeat(2)),
        initiator_subject: "remote".to_owned(),
        initiator_suite_subject: peer_subject,
        initiator_display_name: None,
    };
    let response = service.exchange(request.clone(), Utc::now()).await?;
    Ok((service.get(owner, response.link_id).await?, request))
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn two_routers_link_both_roles_reconnect_relay_and_disconnect() -> TestResult {
    let db = common::postgres_database().await?;
    let pool = &db.pool;
    let a_user = user(pool, "suite-a").await?;
    let b_user = user(pool, "suite-b").await?;
    let a_token = common::TestTokenIssuer::new().user_token(a_user)?;
    let b_token = common::TestTokenIssuer::new().user_token(b_user)?;
    let a_listener = TcpListener::bind("127.0.0.1:0").await?;
    let b_listener = TcpListener::bind("127.0.0.1:0").await?;
    let a_port = a_listener.local_addr()?.port();
    let b_port = b_listener.local_addr()?.port();
    let a_context = make_context(pool, registry("alpha", "beta", a_port, b_port));
    let b_context = make_context(pool, registry("beta", "alpha", b_port, a_port));
    let a_api = module(pool, a_context.clone(), Arc::new(Applier::default()));
    let b_api = module(pool, b_context.clone(), Arc::new(Applier::default()));
    let a = common::app_with_suite(pool, a_api.clone());
    let b = common::app_with_suite(pool, b_api.clone());
    let a_server = tokio::spawn({
        let a = a.clone();
        async move { axum::serve(a_listener, a).await }
    });
    let b_server = tokio::spawn({
        let b = b.clone();
        async move { axum::serve(b_listener, b).await }
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut old_links = None;
    for attempt in 0..2 {
        let (status,start)=call(&a,"POST","/suite/links",Some(&a_token),json!({"peerApp":"beta","returnUrl":format!("http://127.0.0.1:{a_port}/suite/linked"),"stateNonce":"client nonce"}))
        .await?;
        assert_eq!(status, StatusCode::CREATED);
        let authorize = start["authorizeUrl"].as_str().expect("valid test value");
        let state = query(authorize, "state");
        let challenge = query(authorize, "code_challenge");
        let (status, preview) = call(
            &b,
            "GET",
            "/suite/authorizations/preview?client=alpha",
            Some(&b_token),
            Value::Null,
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        assert!(!preview["autoApprove"].as_bool().expect("valid test value"));
        let (status, authorized) = call(
            &b,
            "POST",
            "/suite/authorizations",
            Some(&b_token),
            json!({"client":"alpha","state":state,"codeChallenge":challenge}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        let callback = authorized["redirectUrl"]
            .as_str()
            .expect("valid test value");
        let response = http.get(callback).send().await?;
        assert_eq!(response.status(), 303);
        let location = response.headers()["location"].to_str()?;
        assert_eq!(query(location, "state"), "client nonce");
        let code = query(location, "code");
        let request = query(location, "request");
        let wrong = call(
            &a,
            "POST",
            &format!("/suite/links/requests/{request}/complete"),
            Some(&b_token),
            json!({"code":code}),
        )
        .await?;
        assert_eq!(wrong.0, StatusCode::NOT_FOUND);
        assert_eq!(wrong.1["error"]["code"], "not_found");
        let complete_path = format!("/suite/links/requests/{request}/complete");
        let (first, concurrent) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                call(
                    &a,
                    "POST",
                    &complete_path,
                    Some(&a_token),
                    json!({"code":code})
                ),
                call(
                    &a,
                    "POST",
                    &complete_path,
                    Some(&a_token),
                    json!({"code":code})
                )
            )
        })
        .await?;
        let (status, completed) = first?;
        assert_eq!(concurrent?, (status, completed.clone()));
        assert_eq!(status, StatusCode::CREATED, "{completed}");
        let retained: (bool, bool, bool, bool) = sqlx::query_as(
            "SELECT initiator_link_id IS NULL, secret_ciphertext IS NULL, secret_nonce IS NULL,
             secret_key_version IS NULL FROM suite_link_requests WHERE id=$1",
        )
        .bind(Uuid::parse_str(&request)?)
        .fetch_one(pool)
        .await?;
        assert_eq!(retained, (true, true, true, true));
        let retry = call(
            &a,
            "POST",
            &format!("/suite/links/requests/{request}/complete"),
            Some(&a_token),
            json!({"code":code}),
        )
        .await?;
        assert_eq!(retry.0, StatusCode::CREATED);
        assert_eq!(retry.1, completed);
        let a_id = Uuid::parse_str(completed["id"].as_str().expect("valid test value"))?;
        let a_link = a_api.get(a_user, a_id).await?.link;
        let b_link = b_api.get(b_user, a_link.remote_link_id).await?.link;
        assert_eq!(a_link.role, LinkRole::Initiator);
        assert_eq!(b_link.role, LinkRole::Authorizer);
        assert_eq!(a_link.remote_subject, b_user.to_string());
        assert_eq!(b_link.remote_subject, a_user.to_string());
        assert_eq!(a_link.remote_display_name.as_deref(), Some("suite-b"));
        assert_eq!(b_link.remote_display_name.as_deref(), Some("suite-a"));
        assert_eq!(secret(&a_link), secret(&b_link));
        if let Some((old_a, old_b)) = old_links {
            assert_eq!(
                a_context
                    .store
                    .find_link(old_a)
                    .await?
                    .expect("valid test value")
                    .status,
                LinkStatus::Revoked
            );
            assert_eq!(
                b_context
                    .store
                    .find_link(old_b)
                    .await?
                    .expect("valid test value")
                    .status,
                LinkStatus::Revoked
            )
        }
        old_links = Some((a_link.id, b_link.id));
        if attempt == 1 {
            assert_eq!(
                call(
                    &a,
                    "DELETE",
                    &format!("/suite/links/{a_id}"),
                    Some(&a_token),
                    Value::Null
                )
                .await?
                .0,
                StatusCode::NO_CONTENT
            );
            let before: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox WHERE job_type='suite.links.revoke' AND payload->>'link_id'=$1").bind(a_id.to_string()).fetch_one(pool).await?;
            assert_eq!(
                call(
                    &a,
                    "DELETE",
                    &format!("/suite/links/{a_id}"),
                    Some(&a_token),
                    Value::Null
                )
                .await?
                .0,
                StatusCode::NO_CONTENT
            );
            let after: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox WHERE job_type='suite.links.revoke' AND payload->>'link_id'=$1").bind(a_id.to_string()).fetch_one(pool).await?;
            assert_eq!(before, after);
            assert_eq!(
                call(
                    &a,
                    "POST",
                    &complete_path,
                    Some(&a_token),
                    json!({"code":code})
                )
                .await?
                .0,
                StatusCode::NOT_FOUND
            );
            let delivery = SuiteDeliveryService::new(a_context.clone());
            assert_eq!(
                delivery
                    .revoke(SuiteRevokeJobV1::new(a_id), Utc::now())
                    .await?,
                DeliveryAction::Delivered
            );
            assert_eq!(
                b_api.get(b_user, b_link.id).await?.link.status,
                LinkStatus::Revoked
            );
        }
    }
    a_server.abort();
    b_server.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn codes_subjects_supersession_and_expiry_follow_fixtures() -> TestResult {
    let (db, c, owner) = authorizer_fixture().await?;
    let service = SuiteLinkService::new(c.clone());
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/suite-events/v1/link-protocol.json"
    ))?;
    for vector in fixture["errors"].as_array().expect("valid test value") {
        if vector["name"] == "unknown-peer" {
            assert_eq!(
                service
                    .preview(
                        SuiteUser {
                            id: owner,
                            suite_subject: None,
                            display_name: None
                        },
                        vector["request"]["client"]
                            .as_str()
                            .expect("valid test value")
                            .to_owned(),
                        None,
                        None,
                    )
                    .await
                    .expect_err("rejected input")
                    .code(),
                vector["error"].as_str().expect("valid test value")
            );
        } else if vector["name"] == "invalid-return-url" {
            let start: LinkStartRequest = serde_json::from_value(vector["request"].clone())?;
            assert_eq!(
                service
                    .start(
                        SuiteUser {
                            id: owner,
                            suite_subject: None,
                            display_name: None
                        },
                        start,
                        Utc::now()
                    )
                    .await
                    .expect_err("rejected input")
                    .code(),
                vector["error"].as_str().expect("valid test value")
            );
        }
    }
    let (link, mut request) = exchange_link(
        &service,
        owner,
        Some("one|person".to_owned()),
        Some("two|person".to_owned()),
    )
    .await?;
    assert!(link.suite_subject.is_none());
    let retry = service.exchange(request.clone(), Utc::now()).await?;
    assert_eq!(retry.link_id, link.id);
    request.initiator_link_id = Uuid::now_v7();
    assert_eq!(
        service
            .exchange(request.clone(), Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_code_invalid"
    );
    request.code_verifier = URL_SAFE_NO_PAD.encode([10u8; 32]);
    assert_eq!(
        service
            .exchange(request, Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_code_invalid"
    );
    let u = SuiteUser {
        id: owner,
        suite_subject: Some("one|person".to_owned()),
        display_name: None,
    };
    let mismatched = service
        .preview(
            u.clone(),
            "beta".to_owned(),
            Some(hint_for("one|other")),
            Some("one".to_owned()),
        )
        .await?;
    assert!(mismatched.hint_mismatch);
    let verifier = URL_SAFE_NO_PAD.encode([11u8; 32]);
    let issued = service
        .authorize(
            u.clone(),
            AuthorizationRequest {
                client: "beta".to_owned(),
                state: verifier.clone(),
                code_challenge: pkce_challenge(&verifier)?,
                hint: Some(hint_for("one|person")),
            },
            Utc::now(),
        )
        .await?;
    let mut request = ExchangeRequest {
        client: "beta".to_owned(),
        code: query(&issued.redirect_url, "code"),
        code_verifier: verifier.clone(),
        initiator_link_id: Uuid::now_v7(),
        link_secret: verifier,
        initiator_subject: "remote".to_owned(),
        initiator_suite_subject: None,
        initiator_display_name: None,
    };
    assert_eq!(
        service
            .exchange(request.clone(), Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_link_account_mismatch"
    );
    request.initiator_suite_subject = Some("one|other".to_owned());
    assert_eq!(
        service
            .exchange(request.clone(), Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_link_account_mismatch"
    );
    request.initiator_suite_subject = Some("one|person".to_owned());
    let created = service.exchange(request.clone(), Utc::now()).await?;
    assert_eq!(created.suite_subject, Some("one|person".to_owned()));
    assert_eq!(
        service
            .exchange(request, Utc::now() + chrono::Duration::seconds(61))
            .await
            .expect_err("rejected input")
            .code(),
        "suite_code_invalid"
    );
    let start = LinkStartRequest {
        peer_app: "beta".to_owned(),
        return_url: "http://127.0.0.1:12340/suite/linked".to_owned(),
        state_nonce: "nonce".to_owned(),
    };
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_link_requests")
        .fetch_one(&db.pool)
        .await?;
    for nonce in ["", "invalid\nnonce", &"x".repeat(129)] {
        let invalid = LinkStartRequest {
            state_nonce: nonce.into(),
            ..start.clone()
        };
        assert_eq!(
            service
                .start(u.clone(), invalid, Utc::now())
                .await
                .expect_err("nonce")
                .code(),
            "suite_payload_invalid"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM suite_link_requests")
            .fetch_one(&db.pool)
            .await?,
        before
    );
    let other_owner = user(&db.pool, "supersession-other").await?;
    let untouched_owner = service
        .start(
            SuiteUser {
                id: other_owner,
                ..SuiteUser::default()
            },
            start.clone(),
            Utc::now(),
        )
        .await?;
    let untouched_peer = service
        .start(
            u.clone(),
            LinkStartRequest {
                peer_app: "hebkit".into(),
                ..start.clone()
            },
            Utc::now(),
        )
        .await?;
    let first = service.start(u.clone(), start.clone(), Utc::now()).await?;
    let sealed = cipher().encrypt(
        first.request_id,
        &baukit_credential_vault::CredentialSecrets::new().with("link_secret", vec![9_u8; 32])?,
    )?;
    let field = sealed.fields.get("link_secret").expect("valid test value");
    let prepared = PreparedExchange {
        link_id: Uuid::now_v7(),
        secret: EncryptedPayload {
            ciphertext: field.ciphertext.clone(),
            nonce: field.nonce.clone(),
            key_version: sealed.key_version,
        },
    };
    let mut tx = c.store.begin_transaction().await?;
    c.store
        .prepare_exchange_in_transaction(&mut tx, first.request_id, owner, &prepared)
        .await?;
    c.store.commit_transaction(tx).await?;
    let second = service.start(u.clone(), start.clone(), Utc::now()).await?;
    for untouched in [untouched_owner.request_id, untouched_peer.request_id] {
        let open: bool =
            sqlx::query_scalar("SELECT consumed_at IS NULL FROM suite_link_requests WHERE id=$1")
                .bind(untouched)
                .fetch_one(&db.pool)
                .await?;
        assert!(open, "supersession changed another owner or peer");
    }
    let cleared: bool = sqlx::query_scalar(
        "SELECT consumed_at IS NOT NULL AND initiator_link_id IS NULL AND \
         secret_ciphertext IS NULL AND secret_nonce IS NULL AND secret_key_version IS NULL \
         FROM suite_link_requests WHERE id=$1",
    )
    .bind(first.request_id)
    .fetch_one(&db.pool)
    .await?;
    assert!(
        cleared,
        "supersession must discard the prepared exchange secret"
    );
    assert_eq!(
        service
            .complete(u.clone(), first.request_id, "code".to_owned(), Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_code_invalid"
    );
    for wrong_source in ["unknown", "hebkit"] {
        let bad = service
            .callback(
                LinkCallbackQuery {
                    state: Some(query(&second.authorize_url, "state")),
                    from: Some(wrong_source.to_owned()),
                    code: Some("code".to_owned()),
                    error: None,
                },
                Utc::now(),
            )
            .await?;
        assert_eq!(query(&bad.location, "status"), "failed");
        assert_eq!(query(&bad.location, "code"), "suite_code_invalid");
    }
    let expired = service
        .callback(
            LinkCallbackQuery {
                state: Some(query(&second.authorize_url, "state")),
                from: Some("beta".to_owned()),
                code: Some("code".to_owned()),
                error: None,
            },
            Utc::now() + chrono::Duration::minutes(11),
        )
        .await?;
    assert_eq!(query(&expired.location, "status"), "failed");
    let mut bad_start = start.clone();
    bad_start.peer_app = "unknown".to_owned();
    assert_eq!(
        service
            .start(u.clone(), bad_start, Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_peer_unknown"
    );
    let mut bad_start = start;
    bad_start.return_url = "https://evil.example/suite/linked".to_owned();
    assert_eq!(
        service
            .start(u, bad_start, Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "suite_return_url_invalid"
    );
    let linked_replays: i64 =
        sqlx::query_scalar("SELECT count(*) FROM suite_links WHERE last_replay_at IS NOT NULL")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(linked_replays, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn exchange_client_length_is_bounded_before_peer_lookup() -> TestResult {
    let (db, context, _) = authorizer_fixture().await?;
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context, Arc::new(Applier::default())),
    );
    let token = URL_SAFE_NO_PAD.encode([9_u8; 32]);
    for length in [
        SUITE_MAX_PEER_ID_CHARACTERS - 1,
        SUITE_MAX_PEER_ID_CHARACTERS,
        SUITE_MAX_PEER_ID_CHARACTERS + 1,
    ] {
        let (status, body) = call(
            &app,
            "POST",
            "/suite/links/exchange",
            None,
            json!({
                "client": "é".repeat(length),
                "code": token,
                "codeVerifier": token,
                "initiatorLinkId": Uuid::now_v7(),
                "linkSecret": token,
                "initiatorSubject": "remote"
            }),
        )
        .await?;
        let (expected_status, expected_code) = if length > SUITE_MAX_PEER_ID_CHARACTERS {
            (StatusCode::UNPROCESSABLE_ENTITY, "suite_payload_invalid")
        } else {
            (StatusCode::BAD_REQUEST, "suite_peer_unknown")
        };
        assert_eq!(status, expected_status);
        assert_eq!(body["error"]["code"], expected_code);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn outbox_rollbacks_xp_signatures_retry_and_terminal_health() -> TestResult {
    let (db, c, owner) = authorizer_fixture().await?;
    let service = SuiteLinkService::new(c.clone());
    let (link, _) = exchange_link(&service, owner, None, None).await?;
    let outbox = make_outbox(db.pool.clone(), c.clone(), true);
    let event = activity(3, Some(17));
    let mut event = event;
    event.natural_key = (Utc::now().date_naive()).to_string();
    let mut tx = c.store.begin_transaction().await?;
    assert_eq!(
        outbox
            .enqueue_in_transaction(&mut tx, owner, std::slice::from_ref(&event))
            .await?,
        1
    );
    tx.rollback().await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_outbox WHERE job_type='suite.events.deliver'")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(count, 0);
    let mut tx = c.store.begin_transaction().await?;
    assert_eq!(
        outbox
            .enqueue_in_transaction(&mut tx, owner, std::slice::from_ref(&event))
            .await?,
        1
    );
    assert_eq!(
        outbox
            .enqueue_in_transaction(&mut tx, owner, std::slice::from_ref(&event))
            .await?,
        0
    );
    c.store.commit_transaction(tx).await?;
    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM job_outbox WHERE job_type='suite.events.deliver'")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(payload["envelope"]["payload"]["xp"], 17);
    assert_eq!(payload["envelope"]["userId"], owner.to_string());
    for (index, (global, per_link)) in [(true, false), (false, true), (false, false)]
        .into_iter()
        .enumerate()
    {
        c.store
            .update_preferences(owner, link.id, Some(per_link), None, Utc::now())
            .await?;
        let outbox = make_outbox(db.pool.clone(), c.clone(), global);
        let event = activity(2, Some(9));
        let mut event = event;
        event.natural_key = (Utc::now().date_naive()
            - chrono::Duration::days(i64::try_from(index)? + 1))
        .to_string();
        let mut tx = c.store.begin_transaction().await?;
        outbox
            .enqueue_in_transaction(&mut tx, owner, &[event])
            .await?;
        c.store.commit_transaction(tx).await?;
    }
    let xp_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_outbox WHERE payload->'envelope'->'payload' ? 'xp'",
    )
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(xp_jobs, 1);
    c.store
        .record_delivery(link.id, DeliveryAction::Unauthorized, Utc::now())
        .await?;
    assert_eq!(
        service.get(owner, link.id).await?.status,
        LinkStatus::NeedsAttention
    );
    sqlx::query("UPDATE suite_links SET status='active',delivery_health='healthy' WHERE id=$1")
        .bind(link.id)
        .execute(&db.pool)
        .await?;
    let jobs = PostgresJobStore::new(db.pool.clone());
    for n in 0..20 {
        let job = jobs
            .enqueue(NewJob::new(
                SUITE_EVENTS_DELIVER_JOB_TYPE,
                json!({"link_id":link.id,"n":n}),
                1,
            ))
            .await?
            .job;
        sqlx::query("UPDATE job_outbox
            SET status='failed',last_error='suite_rejected',failure_reason='permanent',updated_at=now()
            WHERE id=$1")
        .bind(job.id)
        .execute(&db.pool)
        .await?;
    }
    let deliveries = c.store.deliveries(owner, link.id, 20).await?;
    assert_eq!(deliveries.len(), 20);
    assert!(
        deliveries
            .iter()
            .all(|delivery| delivery.event_type.is_none()
                && delivery.status == "failed"
                && delivery.last_error_code.as_deref() == Some("suite_rejected"))
    );
    let disabled = service.get(owner, link.id).await?;
    assert_eq!(disabled.consecutive_failures, 20);
    assert_eq!(disabled.delivery_health, DeliveryHealth::Disabled);
    let mut tx = c.store.begin_transaction().await?;
    assert_eq!(
        outbox
            .enqueue_in_transaction(&mut tx, owner, &[event])
            .await?,
        0
    );
    tx.rollback().await?;
    assert_eq!(
        service
            .reenable(owner, link.id, Utc::now())
            .await?
            .consecutive_failures,
        0
    );
    c.store
        .record_delivery(link.id, DeliveryAction::Unauthorized, Utc::now())
        .await?;
    for n in 1..=SUITE_CIRCUIT_FAILURES {
        let job = jobs
            .enqueue(NewJob::new(
                SUITE_EVENTS_DELIVER_JOB_TYPE,
                json!({"link_id":link.id,"attention_failure":n}),
                1,
            ))
            .await?
            .job;
        sqlx::query("UPDATE job_outbox SET status='failed',last_error='suite_rejected',failure_reason='permanent',updated_at=now() WHERE id=$1")
            .bind(job.id).execute(&db.pool).await?;
        let link = service.get(owner, link.id).await?;
        assert_eq!(link.status, LinkStatus::NeedsAttention);
        assert_eq!(link.consecutive_failures, n);
        assert_eq!(
            link.delivery_health,
            if n < SUITE_CIRCUIT_FAILURES {
                DeliveryHealth::NeedsAttention
            } else {
                DeliveryHealth::Disabled
            }
        );
    }
    assert_eq!(
        service
            .reenable(owner, link.id, Utc::now())
            .await
            .expect_err("rejected input")
            .code(),
        "not_found"
    );
    c.store
        .record_delivery(link.id, DeliveryAction::Revoked, Utc::now())
        .await?;
    assert_eq!(
        service.get(owner, link.id).await?.status,
        LinkStatus::Revoked
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn inbound_limits_isolate_known_links_from_unknown_id_traffic() -> TestResult {
    let db = common::postgres_database().await?;
    let mut context = make_context(&db.pool, registry("alpha", "beta", 12340, 12341));
    // Hold the window fixed so database latency cannot replenish the test budget.
    let limiter = Arc::new(FixedWindowLimiter::default());
    Arc::get_mut(&mut context)
        .expect("valid test value")
        .limiter = limiter.clone();
    let service = SuiteLinkService::new(context.clone());
    let applier = Arc::new(Applier::default());
    let app = common::app_with_suite(&db.pool, module(&db.pool, context, applier.clone()));
    let now = Utc::now();
    let mut envelope: EventEnvelope = activity(24, Some(24)).envelope("remote", "beta");
    envelope.event_type = "beta.activity.completed".into();
    envelope.user_id = "remote".to_owned();
    envelope.occurred_at = now;
    let make_request =
        |id: Uuid, event: &EventEnvelope, key: &[u8]| -> Result<Request<Body>, Box<dyn Error>> {
            let body = serde_json::to_vec(event)?;
            let signature = sign_webhook_hmac_sha256(key, now.timestamp(), &event.event_id, &body);
            Ok(Request::builder()
                .method("POST")
                .uri(format!("/api/v1/suite/inbound/{id}"))
                .header("content-type", "application/json")
                .header("x-suite-source", "beta")
                .header("x-suite-delivery-id", &event.event_id)
                .header("x-suite-timestamp", now.timestamp().to_string())
                .header("x-suite-signature", signature)
                .body(Body::from(body))?)
        };
    let mut links = Vec::new();
    for index in 0..25 {
        let owner = user(&db.pool, &format!("suite-inbound-rate-{index}")).await?;
        let (link, _) = exchange_link(&service, owner, None, None).await?;
        let key = secret(&link);
        envelope.event_id = Uuid::new_v5(
            &SUITE_EVENT_NAMESPACE,
            format!("peer-inbound-rate-{index}").as_bytes(),
        )
        .to_string();
        let response = app
            .clone()
            .oneshot(make_request(link.id, &envelope, &key)?)
            .await?;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        links.push((link, key, envelope.clone()));
    }
    let expected: Vec<_> = links
        .iter()
        .map(|(link, _, _)| {
            (
                format!("suite:inbound:{}", link.id),
                120,
                std::time::Duration::from_secs(60),
            )
        })
        .collect();
    assert_eq!(*limiter.calls.lock().expect("calls"), expected);
    assert!(
        !limiter
            .counts
            .lock()
            .expect("counts")
            .keys()
            .any(|key| key.starts_with("suite:inbound:unknown:"))
    );
    assert_eq!(applier.calls.lock().expect("valid test value").len(), 25);
    let (link, key, mut event) = links[0].clone();
    for index in 0..=SUITE_INBOUND_UNKNOWN_LIMIT {
        let response = app
            .clone()
            .oneshot(make_request(Uuid::now_v7(), &event, &key)?)
            .await?;
        if index < SUITE_INBOUND_UNKNOWN_LIMIT {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?,
            )?;
            assert_eq!(body["error"]["code"], "suite_signature_invalid");
        } else {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            let retry_after = response.headers()["retry-after"].to_str()?.parse::<u64>()?;
            assert!((1..=SUITE_RATE_WINDOW_SECONDS).contains(&retry_after));
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?,
            )?;
            assert_eq!(body["error"]["code"], "rate_limited");
        }
    }
    event.event_id = Uuid::new_v5(&SUITE_EVENT_NAMESPACE, b"known-after-unknown-limit").to_string();
    let response = app
        .clone()
        .oneshot(make_request(link.id, &event, &key)?)
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(applier.calls.lock().expect("valid test value").len(), 26);
    for _ in 2..SUITE_INBOUND_LINK_LIMIT {
        let response = app
            .clone()
            .oneshot(make_request(link.id, &event, &key)?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app
        .clone()
        .oneshot(make_request(link.id, &event, &key)?)
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = response.headers()["retry-after"].to_str()?.parse::<u64>()?;
    assert!((1..=SUITE_RATE_WINDOW_SECONDS).contains(&retry_after));
    let (other_link, other_key, other_event) = &links[1];
    let response = app
        .oneshot(make_request(other_link.id, other_event, other_key)?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(applier.calls.lock().expect("valid test value").len(), 26);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn receiving_signature_fixture_matrix_uses_raw_bytes() -> TestResult {
    let (db, c, owner) = authorizer_fixture().await?;
    let service = SuiteLinkService::new(c.clone());
    let (link, _) = exchange_link(&service, owner, None, None).await?;
    let ingest = SuiteIngestService::new(c, Arc::new(Applier::default()));
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/suite-events/v1/signature.json"
    ))?;
    for case in fixture["suiteCases"].as_array().expect("valid test value") {
        let encrypted = cipher().encrypt(
            link.id,
            &baukit_credential_vault::CredentialSecrets::new().with(
                "link_secret",
                case["secret"]
                    .as_str()
                    .expect("valid test value")
                    .as_bytes()
                    .to_vec(),
            )?,
        )?;
        let field = &encrypted.fields["link_secret"];
        sqlx::query("UPDATE suite_links SET secret_ciphertext=$2,secret_nonce=$3,secret_key_version=$4 WHERE id=$1").bind(link.id).bind(&field.ciphertext).bind(&field.nonce).bind(encrypted.key_version).execute(&db.pool).await?;
        let stamp = case["timestamp"]
            .as_i64()
            .expect("valid test value")
            .to_string();
        let headers = SignedHeaders::parse(
            [
                ("content-type", "application/json"),
                ("x-suite-source", "beta"),
                (
                    "x-suite-signature",
                    case["signature"].as_str().expect("valid test value"),
                ),
                ("x-suite-timestamp", stamp.as_str()),
                (
                    "x-suite-delivery-id",
                    case["deliveryId"].as_str().expect("valid test value"),
                ),
            ]
            .into_iter(),
        )?;
        let result = ingest
            .ingest(
                link.id,
                headers,
                case["body"]
                    .as_str()
                    .expect("valid test value")
                    .as_bytes()
                    .to_vec(),
                chrono::DateTime::from_timestamp(
                    case["now"].as_i64().expect("valid test value"),
                    0,
                )
                .expect("valid test value"),
            )
            .await
            .expect_err("rejected input");
        assert_eq!(
            result.code(),
            if case["expected"].as_bool().expect("valid test value") {
                "suite_payload_invalid"
            } else {
                "suite_signature_invalid"
            },
            "{}",
            case["name"]
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_inbound_events")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn worker_retries_then_commits_delivery_and_health_with_its_lease() -> TestResult {
    use baukit_jobs::{WorkerConfig, WorkerRunner};
    use baukit_suite::adapters::jobs::SuiteJobHandler;
    let db = common::postgres_database().await?;
    let receiver = ScriptedWebhookReceiver::start().await?;
    receiver.push_response(ScriptedWebhookResponse::new(503));
    receiver.push_response(ScriptedWebhookResponse::success());
    let port = Url::parse(&receiver.url("/"))?
        .port()
        .expect("valid test value");
    let c = make_context(&db.pool, registry("alpha", "beta", 12340, port));
    let owner = user(&db.pool, "suite-worker").await?;
    let links = SuiteLinkService::new(c.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    let event = activity(1, Some(2));
    let mut event = event;
    event.natural_key = (Utc::now().date_naive()).to_string();
    let mut tx = c.store.begin_transaction().await?;
    c.outbox
        .enqueue_in_transaction(&mut tx, owner, &[event])
        .await?;
    c.store.commit_transaction(tx).await?;
    let shutdown = baukit_runtime::ShutdownToken::new(std::time::Duration::from_secs(5));
    let runner = WorkerRunner::new(
        Arc::new(PostgresJobStore::new(db.pool.clone())),
        Arc::new(SuiteJobHandler::new(Arc::new(SuiteDeliveryService::new(
            c.clone(),
        )))),
        WorkerConfig {
            queue: "suite-events",
            worker_id: "suite-test-worker".to_owned(),
            poll_interval: std::time::Duration::from_millis(10),
            retry_initial: std::time::Duration::from_millis(10),
            retry_max: std::time::Duration::from_millis(20),
            ..WorkerConfig::default()
        },
    )?;
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let status: String = sqlx::query_scalar(
                "SELECT status FROM job_outbox WHERE job_type='suite.events.deliver'",
            )
            .fetch_one(&db.pool)
            .await?;
            if status == "succeeded" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<(), sqlx::Error>(())
    })
    .await??;
    shutdown.trigger();
    task.await??;
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM job_outbox WHERE job_type='suite.events.deliver'")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(attempts, 2);
    let received = receiver.received_requests();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].body(), received[1].body());
    let key = secret(&link);
    for request in &received {
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.header("x-suite-source"), Some("alpha"));
        assert_eq!(request.header("x-suite-replay"), None);
        assert!(verify_webhook_hmac_sha256(
            [key.as_slice()],
            request
                .header("x-suite-timestamp")
                .expect("valid test value")
                .parse()?,
            request
                .header("x-suite-delivery-id")
                .expect("valid test value"),
            request.body(),
            request
                .header("x-suite-signature")
                .expect("valid test value")
        ));
    }
    let fresh = links.get(owner, link.id).await?;
    assert_eq!(fresh.delivery_health, DeliveryHealth::Healthy);
    assert!(fresh.last_delivery_at.is_some());
    assert_eq!(fresh.consecutive_failures, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn cleanup_removes_old_terminal_suite_jobs_and_retains_recent_jobs() -> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    let now = Utc::now();
    context
        .store
        .record_delivery(link.id, DeliveryAction::Delivered, now)
        .await?;
    let jobs = PostgresJobStore::new(db.pool.clone());
    let mut retained = Vec::new();
    for old in [true, false] {
        for (kind, event_type) in [
            (SUITE_EVENTS_DELIVER_JOB_TYPE, "alpha.activity.completed"),
            (SUITE_EVENTS_DELIVER_JOB_TYPE, "suite.connection.tested"),
            (SUITE_LINKS_REVOKE_JOB_TYPE, ""),
        ] {
            for status in ["succeeded", "failed"] {
                let updated = if old {
                    now - chrono::Duration::days(31)
                } else {
                    now - chrono::Duration::days(29)
                };
                let mut pending = NewJob::new(
                    kind,
                    json!({"link_id":link.id,"envelope":{"type":event_type}}),
                    1,
                );
                pending.created_at = updated;
                pending.run_after = updated;
                let job = jobs.enqueue(pending).await?.job;
                sqlx::query(
                    "UPDATE job_outbox SET status=$2,updated_at=$3,failure_reason=$4 WHERE id=$1",
                )
                .bind(job.id)
                .bind(status)
                .bind(updated)
                .bind((status == "failed").then_some("permanent"))
                .execute(&db.pool)
                .await?;
                if !old {
                    retained.push(job.id);
                }
            }
        }
    }
    let result = context.store.cleanup(now).await?;
    assert_eq!(result.terminal_jobs, 6);
    let mut remaining: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM job_outbox")
        .fetch_all(&db.pool)
        .await?;
    remaining.sort();
    retained.sort();
    assert_eq!(remaining, retained);
    assert!(context.store.find_link(link.id).await?.is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn unavailable_appliers_and_corrupt_storage_fail_with_retryable_or_internal_errors()
-> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let links = SuiteLinkService::new(context.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    let mut event: EventEnvelope = activity(24, Some(24)).envelope("remote", "beta");
    event.event_type = "beta.activity.completed".into();
    event.user_id = "remote".to_owned();
    event.occurred_at = Utc::now();
    let raw = serde_json::to_vec(&event)?;
    let request = || {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/suite/inbound/{}", link.id))
            .header("content-type", "application/json; charset=utf-8")
            .header("x-suite-source", "beta")
            .header("x-suite-delivery-id", &event.event_id)
            .header(
                "x-suite-timestamp",
                event.occurred_at.timestamp().to_string(),
            )
            .header(
                "x-suite-signature",
                sign_webhook_hmac_sha256(
                    &secret(&link),
                    event.occurred_at.timestamp(),
                    &event.event_id,
                    &raw,
                ),
            )
            .body(Body::from(raw.clone()))
            .expect("valid test value")
    };
    let unavailable = common::app_with_suite(
        &db.pool,
        Arc::new(SuiteModule {
            links: SuiteLinkService::new(context.clone()),
            delivery: SuiteDeliveryService::new(context.clone()),
            ingest: SuiteIngestService::with_optional_applier(context.clone(), None),
        }),
    );
    let response = unavailable.oneshot(request()).await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["retry-after"], "300");
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?,
    )?;
    assert_eq!(body["error"]["code"], "suite_unavailable");
    let corrupt = common::app_with_suite(
        &db.pool,
        module(
            &db.pool,
            context.clone(),
            Arc::new(Applier {
                invalid: true,
                ..Applier::default()
            }),
        ),
    );
    let response = corrupt.clone().oneshot(request()).await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?,
    )?;
    assert_eq!(body["error"]["code"], "internal_server_error");
    let inbox: i64 =
        sqlx::query_scalar("SELECT count(*) FROM suite_inbound_events WHERE link_id=$1")
            .bind(link.id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(inbox, 0);
    let writes: i64 = sqlx::query_scalar("SELECT count(*) FROM applied_events")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(writes, 0);
    let started = links
        .start(
            SuiteUser {
                id: owner,
                suite_subject: None,
                display_name: None,
            },
            LinkStartRequest {
                peer_app: "beta".to_owned(),
                state_nonce: "nonce".to_owned(),
                return_url: "http://127.0.0.1:12340/suite/linked".to_owned(),
            },
            Utc::now(),
        )
        .await?;
    sqlx::query("UPDATE suite_link_requests SET state_hash='\\x01'::bytea WHERE id=$1")
        .bind(started.request_id)
        .execute(&db.pool)
        .await?;
    let token = common::TestTokenIssuer::new().user_token(owner)?;
    let (status, body) = call(
        &corrupt,
        "POST",
        &format!("/suite/links/requests/{}/complete", started.request_id),
        Some(&token),
        json!({"code":"unused-code"}),
    )
    .await?;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "internal_server_error");
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn disabled_jobs_fail_permanently_and_revoked_failures_leave_link_health_unchanged()
-> TestResult {
    use baukit_jobs::{WorkerConfig, WorkerRunner};
    use baukit_suite::adapters::jobs::SuiteJobHandler;
    let db = common::postgres_database().await?;
    let receiver = ScriptedWebhookReceiver::start().await?;
    let port = Url::parse(&receiver.url("/"))?
        .port()
        .expect("valid test value");
    let context = make_context(&db.pool, registry("alpha", "beta", 12340, port));
    let owner = user(&db.pool, "suite-disabled-worker").await?;
    let links = SuiteLinkService::new(context.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    SuiteDeliveryService::new(context.clone())
        .test(owner, link.id, Utc::now())
        .await?;
    sqlx::query(
        "UPDATE suite_links SET delivery_health='disabled',consecutive_failures=20,last_failure_code='suite_unauthorized' WHERE id=$1",
    )
    .bind(link.id)
    .execute(&db.pool)
    .await?;
    let token = common::TestTokenIssuer::new().user_token(owner)?;
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context.clone(), Arc::new(Applier::default())),
    );
    for operation in ["test", "replay"] {
        let body = if operation == "replay" {
            json!({"since":Utc::now().date_naive().to_string()})
        } else {
            json!({})
        };
        let (status, body) = call(
            &app,
            "POST",
            &format!("/suite/links/{}/{operation}", link.id),
            Some(&token),
            body,
        )
        .await?;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "suite_link_inactive");
    }
    let shutdown = baukit_runtime::ShutdownToken::new(std::time::Duration::from_secs(5));
    let runner = WorkerRunner::new(
        Arc::new(PostgresJobStore::new(db.pool.clone())),
        Arc::new(SuiteJobHandler::new(Arc::new(SuiteDeliveryService::new(
            context.clone(),
        )))),
        WorkerConfig {
            queue: "suite-events",
            poll_interval: std::time::Duration::from_millis(10),
            ..WorkerConfig::default()
        },
    )?;
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let status: String = sqlx::query_scalar(
                "SELECT status FROM job_outbox WHERE job_type='suite.events.deliver'",
            )
            .fetch_one(&db.pool)
            .await?;
            if status == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<(), sqlx::Error>(())
    })
    .await??;
    shutdown.trigger();
    task.await??;
    let history = context.store.deliveries(owner, link.id, 20).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "failed");
    assert_eq!(
        history[0].last_error_code.as_deref(),
        Some("suite_link_disabled")
    );
    assert_eq!(
        links
            .get(owner, link.id)
            .await?
            .last_failure_code
            .as_deref(),
        Some("suite_unauthorized")
    );
    assert_eq!(receiver.calls(), 0);
    links.disconnect(owner, link.id, Utc::now()).await?;
    let before = links.get(owner, link.id).await?;
    let jobs = PostgresJobStore::new(db.pool.clone());
    for id in [link.id.to_string(), "malformed-uuid".to_owned()] {
        let job = jobs
            .enqueue(NewJob::new(
                SUITE_EVENTS_DELIVER_JOB_TYPE,
                json!({"link_id":id}),
                1,
            ))
            .await?
            .job;
        sqlx::query("UPDATE job_outbox SET status='failed',last_error='suite_link_revoked',failure_reason='permanent',updated_at=now() WHERE id=$1").bind(job.id).execute(&db.pool).await?;
    }
    let after = links.get(owner, link.id).await?;
    assert_eq!(after.consecutive_failures, before.consecutive_failures);
    assert_eq!(after.last_failure_code, before.last_failure_code);
    assert_eq!(after.delivery_health, before.delivery_health);
    for operation in ["test", "replay"] {
        let body = if operation == "replay" {
            json!({"since":Utc::now().date_naive().to_string()})
        } else {
            json!({})
        };
        let (status, body) = call(
            &app,
            "POST",
            &format!("/suite/links/{}/{operation}", link.id),
            Some(&token),
            body,
        )
        .await?;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(body["error"]["code"], "suite_link_revoked");
    }
    Ok(())
}
const PEERS: &str = include_str!("../../../../fixtures/suite-events/v1/peers.json");
const CATALOG: &str = include_str!("../../../../fixtures/suite-events/v1/catalog.json");
struct Identities(PgPool);
#[async_trait]
impl SuiteIdentitySource for Identities {
    async fn identity(&self, owner: Uuid) -> Result<SuiteIdentity, SuiteStoreError> {
        let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id=$1")
            .bind(owner)
            .fetch_one(&self.0)
            .await
            .map_err(|error| SuiteStoreError::Storage(error.to_string()))?;
        Ok(SuiteIdentity {
            subject: owner.to_string(),
            display_name: Some(name),
        })
    }
    async fn identity_in_transaction(
        &self,
        connection: &mut sqlx::PgConnection,
        owner: Uuid,
    ) -> Result<SuiteIdentity, SuiteStoreError> {
        let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id=$1")
            .bind(owner)
            .fetch_one(connection)
            .await
            .map_err(|error| SuiteStoreError::Storage(error.to_string()))?;
        Ok(SuiteIdentity {
            subject: owner.to_string(),
            display_name: Some(name),
        })
    }
}
#[derive(Default)]
struct History {
    reads: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl SuiteReplaySource for History {
    async fn replay_since(
        &self,
        connection: &mut sqlx::PgConnection,
        owner: Uuid,
        since: chrono::NaiveDate,
    ) -> Result<Vec<SuiteEvent>, SuiteStoreError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let events: Vec<(String, String, chrono::DateTime<Utc>, Value)> = sqlx::query_as("SELECT event_type,natural_key,occurred_at,payload FROM history_events WHERE owner_id=$1 AND occurred_at::date >= $2 ORDER BY occurred_at,natural_key")
            .bind(owner).bind(since).fetch_all(connection).await.map_err(|error| SuiteStoreError::Storage(error.to_string()))?;
        events
            .into_iter()
            .map(|(event_type, natural_key, occurred_at, payload)| {
                Ok(SuiteEvent {
                    event_type,
                    natural_key,
                    occurred_at,
                    payload: serde_json::from_value(payload)
                        .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?,
                })
            })
            .collect()
    }
}
fn make_context(pool: &PgPool, registry: Arc<PeerRegistry>) -> Arc<SuiteContext> {
    let catalog = Arc::new(PayloadCatalog::from_json(CATALOG).expect("catalog"));
    let identities = Arc::new(Identities(pool.clone()));
    Arc::new(SuiteContext::new(
        registry.clone(),
        catalog.clone(),
        SuiteDependencies {
            store: Arc::new(PostgresSuiteLinkStore::new(pool.clone())),
            outbox: Arc::new(PostgresSuiteEventOutbox::new(
                pool.clone(),
                registry,
                catalog,
                identities.clone(),
                true,
                "test".into(),
            )),
            peer: Arc::new(ReqwestSuitePeerClient::new(true).expect("client")),
            limiter: Arc::new(InMemoryRateLimitStore::default()),
            replay: Arc::new(History::default()),
            identities,
        },
        SuiteServiceConfig {
            metric_prefix: "test".into(),
            cipher: Some(cipher()),
            initial_replay_days: 90,
            share_xp: true,
        },
    ))
}
fn make_outbox(
    pool: PgPool,
    context: Arc<SuiteContext>,
    share_xp: bool,
) -> PostgresSuiteEventOutbox {
    PostgresSuiteEventOutbox::new(
        pool,
        context.registry.clone(),
        context.catalog.clone(),
        context.identities.clone(),
        share_xp,
        "test".into(),
    )
}
fn module(
    _pool: &PgPool,
    context: Arc<SuiteContext>,
    applier: Arc<dyn SuiteEventApplier>,
) -> Arc<dyn SuiteApi> {
    Arc::new(SuiteModule {
        links: SuiteLinkService::new(context.clone()),
        delivery: SuiteDeliveryService::new(context.clone()),
        ingest: SuiteIngestService::new(context, applier),
    })
}
fn activity(minutes: u32, xp: Option<u32>) -> SuiteEvent {
    let id = Uuid::now_v7();
    let mut payload = json!({"activityId": id, "minutes": minutes})
        .as_object()
        .expect("map")
        .clone();
    if let Some(xp) = xp {
        payload.insert("xp".into(), json!(xp));
    }
    SuiteEvent {
        event_type: "alpha.activity.completed".into(),
        natural_key: id.to_string(),
        occurred_at: Utc::now().trunc_subsecs(6),
        payload,
    }
}
mod common {
    use super::*;
    pub struct TestDatabase {
        pub pool: PgPool,
        pub _container: baukit_test::PostgresTestContainer,
    }
    pub async fn postgres_database() -> Result<TestDatabase, Box<dyn Error>> {
        let database = postgres_database_before_lock_order().await?;
        sqlx::raw_sql(SUITE_LOCK_ORDER_MIGRATION_SQL)
            .execute(&database.pool)
            .await?;
        Ok(database)
    }
    pub async fn postgres_database_before_lock_order() -> Result<TestDatabase, Box<dyn Error>> {
        let container = baukit_test::start_postgres().await?;
        let pool = PgPool::connect(container.connection_url()).await?;
        for migration in [
            baukit_jobs::POSTGRES_MIGRATION_SQL,
            baukit_jobs::POSTGRES_MIGRATION_0002_SQL,
            baukit_jobs::POSTGRES_MIGRATION_0003_SQL,
            SUITE_MIGRATION_SQL,
            SUITE_RUNTIME_MIGRATION_SQL,
        ] {
            sqlx::raw_sql(migration).execute(&pool).await?;
        }
        sqlx::raw_sql("CREATE TABLE users (id uuid PRIMARY KEY, email text NOT NULL, username text NOT NULL, display_name text NOT NULL); CREATE TABLE applied_events (owner_id uuid NOT NULL, event_id text NOT NULL, replay boolean NOT NULL, reward_mode text NOT NULL, payload jsonb NOT NULL, PRIMARY KEY(owner_id,event_id)); CREATE TABLE history_events(owner_id uuid NOT NULL,event_type text NOT NULL,natural_key text NOT NULL,occurred_at timestamptz NOT NULL,payload jsonb NOT NULL);").execute(&pool).await?;
        Ok(TestDatabase {
            pool,
            _container: container,
        })
    }
    pub struct TestTokenIssuer;
    impl TestTokenIssuer {
        pub fn new() -> Self {
            Self
        }
        pub fn user_token(&self, owner: Uuid) -> Result<String, Box<dyn Error>> {
            Ok(owner.to_string())
        }
    }
    async fn principal(
        mut request: Request<Body>,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        if let Some(owner) = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|value| Uuid::parse_str(value).ok())
        {
            request.extensions_mut().insert(SuiteUser {
                id: owner,
                suite_subject: None,
                display_name: None,
            });
        }
        next.run(request).await
    }
    pub fn app_with_suite(_pool: &PgPool, api: Arc<dyn SuiteApi>) -> Router {
        Router::new()
            .nest(
                "/api/v1",
                baukit_suite::adapters::http::router(
                    baukit_suite::adapters::http::SuiteHttpState { api },
                ),
            )
            .layer(axum::middleware::from_fn(principal))
    }
}

#[derive(Default)]
struct FixedWindowLimiter {
    calls: Mutex<Vec<(String, u64, std::time::Duration)>>,
    counts: Mutex<BTreeMap<String, u64>>,
}
impl baukit_ratelimit::RateLimitStore for FixedWindowLimiter {
    fn check_and_consume<'a>(
        &'a self,
        key: &'a str,
        quota: baukit_ratelimit::Quota,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        baukit_ratelimit::RateLimitDecision,
                        baukit_ratelimit::RateLimitStoreError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push((key.into(), quota.capacity(), quota.period()));
            let mut counts = self.counts.lock().expect("limiter");
            let count = counts.entry(key.into()).or_default();
            let allowed = *count < quota.capacity();
            if allowed {
                *count += 1;
            }
            Ok(baukit_ratelimit::RateLimitDecision {
                allowed,
                remaining: quota.capacity().saturating_sub(*count),
                retry_after: if allowed {
                    std::time::Duration::ZERO
                } else {
                    quota.period()
                },
            })
        })
    }
}

async fn signed_route(
    app: &Router,
    id: Uuid,
    event: &EventEnvelope,
    key: &[u8],
    replay: bool,
) -> Result<(StatusCode, Value, axum::http::HeaderMap), Box<dyn Error>> {
    let raw = serde_json::to_vec(event)?;
    let timestamp = Utc::now().timestamp();
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/suite/inbound/{id}"))
        .header("content-type", "application/json")
        .header("x-suite-source", &event.source_app)
        .header("x-suite-delivery-id", &event.event_id)
        .header("x-suite-timestamp", timestamp.to_string())
        .header(
            "x-suite-signature",
            sign_webhook_hmac_sha256(key, timestamp, &event.event_id, &raw),
        );
    if replay {
        request = request.header("x-suite-replay", "1");
    }
    let response = app.clone().oneshot(request.body(Body::from(raw))?).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?;
    Ok((status, serde_json::from_slice(&body)?, headers))
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn receiving_deduplicates_conflicts_isolates_owners_and_makes_replay_rewardless() -> TestResult
{
    let (db, context, owner) = authorizer_fixture().await?;
    let links = SuiteLinkService::new(context.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    let other = user(&db.pool, "other-owner").await?;
    let (other_link, _) = exchange_link(&links, other, None, None).await?;
    let applier = Arc::new(Applier::default());
    let app = common::app_with_suite(&db.pool, module(&db.pool, context.clone(), applier.clone()));
    let mut event = activity(24, Some(24)).envelope("remote", "beta");
    event.event_type = "beta.activity.completed".into();
    let key = secret(&link);
    for (status, outcome) in [
        (StatusCode::ACCEPTED, "granted"),
        (StatusCode::OK, "duplicate"),
    ] {
        let response = signed_route(&app, link.id, &event, &key, false).await?;
        assert_eq!(response.0, status);
        assert_eq!(response.1["outcome"], outcome);
    }
    assert_eq!(applier.calls.lock().expect("calls").len(), 1);
    let wrong_key = signed_route(&app, other_link.id, &event, &key, false).await?;
    assert_eq!(wrong_key.0, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_key.1["error"]["code"], "suite_signature_invalid");
    event.payload.insert("minutes".into(), json!(25));
    let conflict = signed_route(&app, link.id, &event, &key, false).await?;
    assert_eq!(conflict.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(conflict.1["error"]["code"], "suite_event_id_conflict");
    let other_response =
        signed_route(&app, other_link.id, &event, &secret(&other_link), false).await?;
    assert_eq!(other_response.0, StatusCode::ACCEPTED);
    event.event_id = Uuid::new_v5(&SUITE_EVENT_NAMESPACE, b"replayed").to_string();
    event.occurred_at = Utc::now() - chrono::Duration::days(200);
    let stale = signed_route(&app, link.id, &event, &key, false).await?;
    assert_eq!(stale.1["error"]["code"], "event_too_old");
    let replay = signed_route(&app, link.id, &event, &key, true).await?;
    assert_eq!(replay.0, StatusCode::ACCEPTED);
    assert_eq!(replay.1["outcome"], "no_rule");
    assert_eq!(replay.1["ledgerEntryId"], Value::Null);
    let calls = applier.calls.lock().expect("calls").clone();
    assert_eq!(
        calls,
        vec![
            (owner, RewardMode::Native, false),
            (other, RewardMode::Native, false),
            (owner, RewardMode::Off, true)
        ]
    );
    let rows: Vec<(Uuid, bool, String)> = sqlx::query_as(
        "SELECT owner_id,replay,reward_mode FROM applied_events ORDER BY owner_id,replay",
    )
    .fetch_all(&db.pool)
    .await?;
    assert_eq!(rows.len(), 3);
    assert!(rows.contains(&(owner, true, "off".into())));
    event.occurred_at = Utc::now();
    event.user_id = "wrong-subject".into();
    assert_eq!(
        signed_route(&app, link.id, &event, &key, false).await?.1["error"]["code"],
        "event_user_mismatch"
    );
    assert_eq!(applier.calls.lock().expect("calls").len(), 3);
    Ok(())
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn initial_replay_reads_product_history_and_standalone_rejects_writes() -> TestResult {
    let mut db = common::postgres_database().await?;
    db.pool.close().await;
    db.pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(db._container.connection_url())
        .await?;
    let owner = user(&db.pool, "history-owner").await?;
    let event = activity(40, Some(8));
    sqlx::query("INSERT INTO history_events(owner_id,event_type,natural_key,occurred_at,payload) VALUES($1,$2,$3,$4,$5)")
        .bind(owner).bind(&event.event_type).bind(&event.natural_key).bind(event.occurred_at).bind(json!(event.payload)).execute(&db.pool).await?;
    let context = make_context(&db.pool, registry("alpha", "beta", 12340, 12341));
    let links = SuiteLinkService::new(context.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    let jobs: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM job_outbox WHERE job_type='suite.events.deliver'")
            .fetch_all(&db.pool)
            .await?;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["replay"], true);
    assert_eq!(
        jobs[0]["envelope"],
        json!(event.envelope(&owner.to_string(), "alpha"))
    );
    assert_eq!(jobs[0]["link_id"], link.id.to_string());
    let mut transaction = db.pool.begin().await?;
    assert_eq!(
        context
            .outbox
            .enqueue_in_transaction(&mut transaction, owner, &[activity(5, None)])
            .await?,
        1
    );
    transaction.rollback().await?;
    let standalone = Arc::new(PeerRegistry::new(
        "alpha",
        PEERS,
        PeerRegistrySettings::default(),
    )?);
    let context = make_context(&db.pool, standalone);
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context, Arc::new(Applier::default())),
    );
    let token = owner.to_string();
    assert_eq!(
        call(&app, "GET", "/suite/peers", Some(&token), Value::Null).await?,
        (StatusCode::OK, json!([]))
    );
    let response = call(
        &app,
        "POST",
        "/suite/links",
        Some(&token),
        json!({"peerApp":"beta","returnUrl":"alpha://suite/linked","stateNonce":"n"}),
    )
    .await?;
    assert_eq!(response.0, StatusCode::FORBIDDEN);
    assert_eq!(response.1["error"]["code"], "suite_disabled");
    Ok(())
}

#[async_trait]
impl SuiteErasureOwnerLookup for Identities {
    async fn owner(&self, subject: &str) -> Result<Option<Uuid>, sqlx::Error> {
        let Ok(owner) = Uuid::parse_str(subject) else {
            return Ok(None);
        };
        sqlx::query_scalar("SELECT id FROM users WHERE id=$1")
            .bind(owner)
            .fetch_optional(&self.0)
            .await
    }
    async fn owner_in_transaction(
        &self,
        connection: &mut sqlx::PgConnection,
        subject: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let Ok(owner) = Uuid::parse_str(subject) else {
            return Ok(None);
        };
        sqlx::query_scalar("SELECT id FROM users WHERE id=$1")
            .bind(owner)
            .fetch_optional(connection)
            .await
    }
    async fn lock_owner_in_transaction(
        &self,
        connection: &mut sqlx::PgConnection,
        owner: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(owner)
            .fetch_optional(connection)
            .await
            .map(|_| ())
    }
}
struct ProductDeletion {
    peer: Arc<ScriptedWebhookReceiver>,
    expected_calls: usize,
}
impl baukit_erasure::ProductErasure for ProductDeletion {
    fn erase<'a>(
        &'a self,
        connection: &'a mut sqlx::PgConnection,
        subject: &'a str,
    ) -> baukit_erasure::ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            assert_eq!(self.peer.calls(), self.expected_calls);
            let owner = Uuid::parse_str(subject)
                .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
            let suite_rows: i64 =
                sqlx::query_scalar("SELECT count(*) FROM suite_links WHERE user_id=$1")
                    .bind(owner)
                    .fetch_one(&mut *connection)
                    .await?;
            assert_eq!(suite_rows, 0);
            sqlx::query("DELETE FROM users WHERE id=$1")
                .bind(owner)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}
struct IdentityDeletion;
impl baukit_erasure::IdentityAccountDeleter for IdentityDeletion {
    fn delete_account<'a>(
        &'a self,
        _: &'a str,
    ) -> baukit_erasure::ErasureFuture<'a, Result<(), baukit_erasure::IdentityDeletionError>> {
        Box::pin(async { Ok(()) })
    }
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn erasure_revokes_every_health_state_first_and_removes_every_suite_row() -> TestResult {
    check_suite_erasure(baukit_erasure::IdentityRetention::Delete {
        deleter: Arc::new(IdentityDeletion),
        provider_id: "test".into(),
        inline_timeout: std::time::Duration::from_millis(500),
        max_attempts: 10,
    })
    .await
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn erasure_retains_shared_identity_and_completes_after_suite_cleanup() -> TestResult {
    check_suite_erasure(baukit_erasure::IdentityRetention::Retain).await
}

async fn check_suite_erasure(identity_retention: baukit_erasure::IdentityRetention) -> TestResult {
    use baukit_suite::services::erase_with_suite;
    let mut db = common::postgres_database().await?;
    db.pool.close().await;
    db.pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(db._container.connection_url())
        .await?;
    sqlx::raw_sql(baukit_erasure::POSTGRES_MIGRATION_SQL)
        .execute(&db.pool)
        .await?;
    let receiver = Arc::new(ScriptedWebhookReceiver::start().await?);
    let port = Url::parse(&receiver.url("/"))?.port().expect("port");
    let context = make_context(&db.pool, registry("alpha", "beta", 12340, port));
    let links = SuiteLinkService::new(context.clone());
    let mut expected_calls = 0;
    for (status, health) in [
        ("active", "healthy"),
        ("needs_attention", "needs_attention"),
        ("active", "disabled"),
        ("revoked", "disabled"),
    ] {
        let owner = user(&db.pool, &format!("erase-{expected_calls}")).await?;
        let (link, _) = exchange_link(&links, owner, None, None).await?;
        let event = activity(10, Some(2));
        let mut connection = db.pool.begin().await?;
        context
            .outbox
            .enqueue_in_transaction(&mut connection, owner, &[event])
            .await?;
        context
            .outbox
            .enqueue_revoke_in_transaction(&mut connection, link.id)
            .await?;
        connection.commit().await?;
        let mut inbound = activity(10, Some(2)).envelope("remote", "beta");
        inbound.event_type = "beta.activity.completed".into();
        let app = common::app_with_suite(
            &db.pool,
            module(&db.pool, context.clone(), Arc::new(Applier::default())),
        );
        assert_eq!(
            signed_route(&app, link.id, &inbound, &secret(&link), false)
                .await?
                .0,
            StatusCode::ACCEPTED
        );
        links
            .start(
                SuiteUser {
                    id: owner,
                    ..SuiteUser::default()
                },
                LinkStartRequest {
                    peer_app: "beta".into(),
                    return_url: "http://127.0.0.1:12340/suite/linked".into(),
                    state_nonce: "nonce".into(),
                },
                Utc::now(),
            )
            .await?;
        sqlx::query("UPDATE suite_links SET status=$2,delivery_health=$3 WHERE id=$1")
            .bind(link.id)
            .bind(status)
            .bind(health)
            .execute(&db.pool)
            .await?;
        receiver.push_response(if status == "revoked" {
            ScriptedWebhookResponse::pending()
        } else {
            ScriptedWebhookResponse::new(503)
        });
        expected_calls += 1;
        let adapter = Arc::new(PostgresSuiteErasure::new(
            PostgresSuiteLinkStore::new(db.pool.clone()),
            Arc::new(Identities(db.pool.clone())),
            Arc::new(ProductDeletion {
                peer: receiver.clone(),
                expected_calls,
            }),
        ));
        let notifier = SuiteErasureNotificationService::new(
            adapter.clone(),
            Arc::new(SuiteDeliveryService::new(context.clone())),
        );
        let service = baukit_erasure::ErasureService::new(
            baukit_erasure::PostgresErasureStore::new(
                db.pool.clone(),
                baukit_config::Secret::new("suite-erasure-key-material-32-bytes".into()),
            )?,
            identity_retention.clone(),
        )?;
        let receipt = erase_with_suite(
            &service,
            &notifier,
            &owner.to_string(),
            &format!("suite-erasure-key-{expected_calls}"),
            adapter.as_ref(),
        )
        .await?;
        assert_eq!(receipt.status, baukit_erasure::ErasureState::Completed);
        assert_eq!(receipt.status_code(), StatusCode::OK);
        assert!(receipt.completed_at.is_some());
        assert_eq!(
            service
                .store()
                .status(&owner.to_string(), receipt.operation_id)
                .await?,
            Some(receipt)
        );
        assert!(service.store().is_fenced(&owner.to_string()).await?);
        let remaining: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM users WHERE id=$1), (SELECT count(*) FROM job_outbox WHERE job_type='identity.account.delete')")
            .bind(owner).fetch_one(&db.pool).await?;
        assert_eq!(remaining, (0, 0));
        let rows: (i64,i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM suite_links WHERE user_id=$1),(SELECT count(*) FROM suite_link_codes WHERE user_id=$1),(SELECT count(*) FROM suite_link_requests WHERE user_id=$1),(SELECT count(*) FROM suite_inbound_events WHERE user_id=$1),(SELECT count(*) FROM suite_failed_delivery_jobs),(SELECT count(*) FROM job_outbox WHERE job_type IN ('suite.events.deliver','suite.links.revoke'))").bind(owner).fetch_one(&db.pool).await?;
        assert_eq!(rows, (0, 0, 0, 0, 0, 0));
        let request = receiver.received_requests().last().expect("revoke").clone();
        assert!(
            request
                .uri()
                .path()
                .ends_with(&format!("/suite/links/{}/revoke", link.remote_link_id))
        );
        assert_eq!(request.header("x-suite-source"), Some("alpha"));
        assert!(verify_webhook_hmac_sha256(
            [secret(&link).as_slice()],
            request
                .header("x-suite-timestamp")
                .expect("timestamp")
                .parse()?,
            request.header("x-suite-delivery-id").expect("id"),
            request.body(),
            request.header("x-suite-signature").expect("signature")
        ));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn in_process_peer_links_the_second_app_and_delivers_to_its_real_router() -> TestResult {
    use baukit_test::suite::{InProcessSuitePeer, fixture};
    assert_eq!(fixture("peers.json"), Some(PEERS));
    assert!(fixture("missing.json").is_none());
    let db = common::postgres_database().await?;
    let a_owner = user(&db.pool, "in-process-a").await?;
    let b_owner = user(&db.pool, "in-process-b").await?;
    let to_a = Arc::new(InProcessSuitePeer::new());
    let to_b = Arc::new(InProcessSuitePeer::new());
    let mut a_context = (*make_context(&db.pool, registry("alpha", "beta", 12340, 12341))).clone();
    let mut b_context = (*make_context(&db.pool, registry("beta", "alpha", 12341, 12340))).clone();
    a_context.peer = to_b.clone();
    b_context.peer = to_a.clone();
    let a_context = Arc::new(a_context);
    let b_context = Arc::new(b_context);
    let b_applier = Arc::new(Applier::default());
    let a_api = module(&db.pool, a_context.clone(), Arc::new(Applier::default()));
    let b_api = module(&db.pool, b_context.clone(), b_applier.clone());
    to_a.mount(baukit_suite::adapters::http::router(
        baukit_suite::adapters::http::SuiteHttpState { api: a_api.clone() },
    ))?;
    to_b.mount(baukit_suite::adapters::http::router(
        baukit_suite::adapters::http::SuiteHttpState { api: b_api.clone() },
    ))?;
    let a = common::app_with_suite(&db.pool, a_api.clone());
    let b = common::app_with_suite(&db.pool, b_api.clone());
    let start = call(
        &a,
        "POST",
        "/suite/links",
        Some(&a_owner.to_string()),
        json!({"peerApp":"beta","returnUrl":"alpha://suite/linked","stateNonce":"client-nonce"}),
    )
    .await?;
    assert_eq!(start.0, StatusCode::CREATED);
    let url = start.1["authorizeUrl"].as_str().expect("authorization URL");
    let authorization = call(&b, "POST", "/suite/authorizations", Some(&b_owner.to_string()), json!({"client":"alpha","state":query(url,"state"),"codeChallenge":query(url,"code_challenge")})).await?;
    assert_eq!(authorization.0, StatusCode::OK);
    let callback = authorization.1["redirectUrl"].as_str().expect("callback");
    let complete = call(
        &a,
        "POST",
        &format!(
            "/suite/links/requests/{}/complete",
            start.1["requestId"].as_str().expect("request")
        ),
        Some(&a_owner.to_string()),
        json!({"code":query(callback,"code")}),
    )
    .await?;
    assert_eq!(complete.0, StatusCode::CREATED, "{}", complete.1);
    let link = a_api
        .get(
            a_owner,
            Uuid::parse_str(complete.1["id"].as_str().expect("link"))?,
        )
        .await?
        .link;
    let remote = b_api.get(b_owner, link.remote_link_id).await?.link;
    assert_eq!(secret(&link), secret(&remote));
    let event = activity(15, Some(4));
    let delivery = SuiteDeliveryService::new(a_context);
    assert_eq!(
        delivery
            .deliver(
                SuiteDeliverJobV1::new(
                    link.id,
                    event.envelope(&a_owner.to_string(), "alpha"),
                    false
                ),
                Utc::now()
            )
            .await?,
        DeliveryAction::Delivered
    );
    assert_eq!(
        b_applier.calls.lock().expect("calls").as_slice(),
        &[(b_owner, RewardMode::Native, false)]
    );
    a_api.disconnect(a_owner, link.id, Utc::now()).await?;
    assert_eq!(
        delivery
            .revoke(SuiteRevokeJobV1::new(link.id), Utc::now())
            .await?,
        DeliveryAction::Delivered
    );
    assert_eq!(
        b_api.get(b_owner, remote.id).await?.link.status,
        LinkStatus::Revoked
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn unknown_source_is_rejected_without_a_database_read_or_limiter_key() -> TestResult {
    let db = common::postgres_database().await?;
    let limiter = Arc::new(FixedWindowLimiter::default());
    let mut context = (*make_context(&db.pool, registry("alpha", "beta", 12340, 12341))).clone();
    context.limiter = limiter.clone();
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, Arc::new(context), Arc::new(Applier::default())),
    );
    db.pool.close().await;
    let response = signed_route(
        &app,
        Uuid::now_v7(),
        &activity(1, None).envelope("user", "unknown"),
        b"key",
        false,
    )
    .await?;
    assert_eq!(response.0, StatusCode::UNAUTHORIZED);
    assert_eq!(response.1["error"]["code"], "suite_signature_invalid");
    assert!(limiter.counts.lock().expect("counts").is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn replay_errors_peer_metadata_and_failed_exchange_budgets_match_the_contract() -> TestResult
{
    let (db, mut context, owner) = authorizer_fixture().await?;
    let history = Arc::new(History::default());
    Arc::get_mut(&mut context).expect("unique context").replay = history.clone();
    let limiter = Arc::new(FixedWindowLimiter::default());
    Arc::get_mut(&mut context).expect("unique context").limiter = limiter.clone();
    Arc::get_mut(&mut context).expect("unique context").share_xp = false;
    let service = SuiteLinkService::new(context.clone());
    let (link, request) = exchange_link(&service, owner, None, None).await?;
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context.clone(), Arc::new(Applier::default())),
    );
    let token = owner.to_string();
    let peers = call(&app, "GET", "/suite/peers", Some(&token), Value::Null).await?;
    assert_eq!(peers.1[0]["scheme"], "beta");
    assert_eq!(peers.1[0]["webUrl"], "http://127.0.0.1:12341");
    assert_eq!(peers.1[0]["shareXpAvailable"], false);
    assert_eq!(
        peers.1[0]["rewardModes"],
        json!(["native", "source_xp", "off"])
    );
    assert!(peers.1[0].get("mappableMetrics").is_none());
    history.reads.store(0, std::sync::atomic::Ordering::SeqCst);
    let last: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT last_replay_at FROM suite_links WHERE id=$1")
            .bind(link.id)
            .fetch_one(&db.pool)
            .await?;
    let exact = SuiteDeliveryService::new(context.clone())
        .replay(
            owner,
            link.id,
            last.date_naive(),
            last + chrono::Duration::seconds(1),
        )
        .await
        .expect_err("cooldown");
    assert!(matches!(
        exact,
        SuiteServiceError::Store(SuiteStoreError::ReplayTooSoon(86_399))
    ));
    assert_eq!(history.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    let today = Utc::now().date_naive();
    let earliest = today - chrono::Duration::days(i64::from(SUITE_MAX_REPLAY_DAYS));
    let path = format!("/suite/links/{}/replay", link.id);
    for since in [
        earliest - chrono::Duration::days(1),
        today + chrono::Duration::days(1),
    ] {
        let response = call(&app, "POST", &path, Some(&token), json!({"since":since})).await?;
        assert_eq!(response.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.1["error"]["code"], "suite_replay_window");
        assert_eq!(
            response.1["error"]["details"],
            json!({"earliest":earliest,"latest":today})
        );
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1{path}"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({"since":today}))?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = response.headers()["retry-after"].to_str()?.parse()?;
    assert!((1..=u64::try_from(SUITE_REPLAY_INTERVAL_SECONDS)?).contains(&retry));
    assert_eq!(history.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    for _ in 0..SUITE_EXCHANGE_FAILURE_LIMIT {
        let mut wrong = request.clone();
        wrong.code_verifier = URL_SAFE_NO_PAD.encode([11u8; 32]);
        assert_eq!(
            service
                .exchange(wrong, Utc::now())
                .await
                .expect_err("wrong verifier")
                .code(),
            "suite_code_invalid"
        );
    }
    let mut wrong = request.clone();
    wrong.code_verifier = URL_SAFE_NO_PAD.encode([11u8; 32]);
    assert!(matches!(
        service.exchange(wrong, Utc::now()).await,
        Err(SuiteServiceError::RateLimited(_))
    ));
    assert_eq!(
        service.exchange(request, Utc::now()).await?.link_id,
        link.id
    );
    assert_eq!(
        limiter
            .counts
            .lock()
            .expect("counts")
            .get("suite:exchange:beta"),
        Some(&u64::from(SUITE_EXCHANGE_FAILURE_LIMIT))
    );
    service.disconnect(owner, link.id, Utc::now()).await?;
    let mut removed = (*context).clone();
    removed.registry = Arc::new(PeerRegistry::new(
        "alpha",
        PEERS,
        PeerRegistrySettings::default(),
    )?);
    assert!(matches!(
        SuiteDeliveryService::new(Arc::new(removed))
            .replay(owner, link.id, today, Utc::now())
            .await,
        Err(SuiteServiceError::LinkRevoked)
    ));
    assert_eq!(history.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    db.pool.close().await;
    for i in 0..200 {
        let input = ExchangeRequest {
            client: format!("unknown-{i}"),
            code: URL_SAFE_NO_PAD.encode([9u8; 32]),
            code_verifier: URL_SAFE_NO_PAD.encode([9u8; 32]),
            initiator_link_id: Uuid::now_v7(),
            link_secret: URL_SAFE_NO_PAD.encode([9u8; 32]),
            initiator_subject: "remote".into(),
            initiator_suite_subject: None,
            initiator_display_name: None,
        };
        assert_eq!(
            service
                .exchange(input, Utc::now())
                .await
                .expect_err("unknown client")
                .code(),
            "suite_peer_unknown"
        );
    }
    assert_eq!(limiter.counts.lock().expect("counts").len(), 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn ingest_authenticates_before_locking_and_rechecks_revocation_after_the_lock() -> TestResult
{
    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    let applier = Arc::new(Applier::default());
    let app = common::app_with_suite(&db.pool, module(&db.pool, context.clone(), applier.clone()));
    let mut tx = db.pool.begin().await?;
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await?;
    context.store.lock_ingest_user(&mut tx, owner).await?;
    let mut event = activity(5, None).envelope("remote", "beta");
    event.event_type = "beta.activity.completed".into();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        signed_route(&app, link.id, &event, &[99; 32], false),
    )
    .await??;
    assert_eq!(response.0, StatusCode::UNAUTHORIZED);
    let signed_link = link.clone();
    let server = app.clone();
    let request = tokio::spawn(async move {
        signed_route(
            &server,
            signed_link.id,
            &event,
            &secret(&signed_link),
            false,
        )
        .await
        .map_err(|error| error.to_string())
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
            )
            .bind(blocker)
            .fetch_one(&db.pool)
            .await?;
            if waiting {
                return Ok::<(), sqlx::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    sqlx::query("UPDATE suite_links SET status='revoked' WHERE id=$1")
        .bind(link.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let response = request.await?.map_err(std::io::Error::other)?;
    assert_eq!(response.0, StatusCode::GONE);
    assert_eq!(response.1["error"]["code"], "suite_link_revoked");
    assert!(applier.calls.lock().expect("calls").is_empty());
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_inbound_events")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(events, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn signed_http_matrix_hides_revocation_and_protocol_tests_skip_the_applier() -> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let links = SuiteLinkService::new(context.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    let applier = Arc::new(Applier::default());
    let app = common::app_with_suite(&db.pool, module(&db.pool, context.clone(), applier.clone()));
    let event = SuiteEvent::connection_test(Uuid::now_v7(), Utc::now()).envelope("remote", "beta");
    let raw = serde_json::to_vec(&event)?;
    let key = secret(&link);
    for missing in [
        "x-suite-source",
        "x-suite-delivery-id",
        "x-suite-timestamp",
        "x-suite-signature",
        "content-type",
    ] {
        let timestamp = Utc::now().timestamp();
        let stamp = timestamp.to_string();
        let signature = sign_webhook_hmac_sha256(&key, timestamp, &event.event_id, &raw);
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/suite/inbound/{}", link.id));
        for (name, value) in [
            ("content-type", "application/json"),
            ("x-suite-source", "beta"),
            ("x-suite-delivery-id", event.event_id.as_str()),
            ("x-suite-timestamp", stamp.as_str()),
            ("x-suite-signature", signature.as_str()),
        ] {
            if name != missing {
                request = request.header(name, value);
            }
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(raw.clone()))?)
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{missing}");
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?,
        )?;
        assert_eq!(body["error"]["code"], "suite_signature_invalid");
    }
    for delta in [-301, 600] {
        let timestamp = Utc::now().timestamp() + delta;
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/suite/inbound/{}", link.id))
            .header("content-type", "application/json")
            .header("x-suite-source", "beta")
            .header("x-suite-delivery-id", &event.event_id)
            .header("x-suite-timestamp", timestamp.to_string())
            .header(
                "x-suite-signature",
                sign_webhook_hmac_sha256(&key, timestamp, &event.event_id, &raw),
            )
            .body(Body::from(raw.clone()))?;
        let response = app.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), SUITE_MAX_BODY_BYTES).await?,
        )?;
        assert_eq!(body["error"]["code"], "suite_signature_invalid");
    }
    let mut wrong_source = event.clone();
    wrong_source.source_app = "hebkit".into();
    let response = signed_route(&app, link.id, &wrong_source, &key, false).await?;
    assert_eq!(response.0, StatusCode::UNAUTHORIZED);
    assert_eq!(response.1["error"]["code"], "suite_signature_invalid");
    sqlx::query("UPDATE suite_links SET receives='{}',delivery_health='degraded',consecutive_failures=19 WHERE id=$1").bind(link.id).execute(&db.pool).await?;
    assert_eq!(
        signed_route(&app, link.id, &event, &key, false).await?.0,
        StatusCode::ACCEPTED
    );
    let updated = links.get(owner, link.id).await?;
    assert_eq!(updated.delivery_health, DeliveryHealth::Degraded);
    assert_eq!(updated.consecutive_failures, 19);
    assert!(updated.last_received_at.is_some());
    assert!(applier.calls.lock().expect("calls").is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_inbound_events")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(count, 0);
    links.disconnect(owner, link.id, Utc::now()).await?;
    for (key, status, code) in [
        (
            &[99_u8; 32][..],
            StatusCode::UNAUTHORIZED,
            "suite_signature_invalid",
        ),
        (&key[..], StatusCode::GONE, "suite_link_revoked"),
    ] {
        let response = signed_route(&app, link.id, &event, key, false).await?;
        assert_eq!(response.0, status);
        assert_eq!(response.1["error"]["code"], code);
    }
    Ok(())
}

struct ScriptedExchangePeer {
    responses: Mutex<std::collections::VecDeque<Result<ExchangeResponse, SuitePeerError>>>,
    exchanges: Mutex<Vec<ExchangeRequest>>,
    barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    pool: PgPool,
    transport: ReqwestSuitePeerClient,
}
#[async_trait]
impl SuitePeerClient for ScriptedExchangePeer {
    async fn exchange(
        &self,
        _: &ActivePeer,
        input: &ExchangeRequest,
    ) -> Result<ExchangeResponse, SuitePeerError> {
        let connection = self
            .pool
            .acquire()
            .await
            .expect("complete must release its transaction before the peer call");
        drop(connection);
        self.exchanges
            .lock()
            .expect("exchanges")
            .push(input.clone());
        let barrier = self.barrier.lock().expect("barrier").clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        self.responses
            .lock()
            .expect("responses")
            .pop_front()
            .ok_or(SuitePeerError::InvalidResponse)?
    }
    async fn deliver(
        &self,
        peer: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        self.transport.deliver(peer, call).await
    }
    async fn revoke(
        &self,
        peer: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        self.transport.revoke(peer, call).await
    }
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn completion_retries_identical_sealed_credentials_outside_the_transaction_and_rejects_invalid_responses()
-> TestResult {
    let mut db = common::postgres_database().await?;
    db.pool.close().await;
    db.pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(db._container.connection_url())
        .await?;
    let mut context = (*make_context(&db.pool, registry("alpha", "beta", 12340, 12341))).clone();
    let peer = context.registry.active_peer("beta").expect("peer");
    let valid = ExchangeResponse {
        link_id: Uuid::now_v7(),
        subject: "remote".into(),
        suite_subject: None,
        display_name: Some("Remote".into()),
        authorizer_sends: peer.receives.clone(),
        initiator_sends: peer.sends.clone(),
    };
    let scripted = Arc::new(ScriptedExchangePeer {
        pool: db.pool.clone(),
        transport: ReqwestSuitePeerClient::new(true)?,
        responses: Mutex::new(std::collections::VecDeque::from([
            Err(SuitePeerError::Timeout),
            Ok(valid.clone()),
        ])),
        exchanges: Mutex::new(Vec::new()),
        barrier: Mutex::new(None),
    });
    context.peer = scripted.clone();
    let service = SuiteLinkService::new(Arc::new(context));
    let owner = user(&db.pool, "timeout-owner").await?;
    let principal = SuiteUser {
        id: owner,
        ..SuiteUser::default()
    };
    let start = service
        .start(
            principal.clone(),
            LinkStartRequest {
                peer_app: "beta".into(),
                return_url: "alpha://suite/linked".into(),
                state_nonce: "nonce".into(),
            },
            Utc::now(),
        )
        .await?;
    let code = URL_SAFE_NO_PAD.encode([9u8; 32]);
    let completed = service
        .complete(
            principal.clone(),
            start.request_id,
            code.clone(),
            Utc::now(),
        )
        .await?;
    assert_eq!(completed.remote_link_id, valid.link_id);
    let exchanges = scripted.exchanges.lock().expect("exchanges").clone();
    assert_eq!(exchanges.len(), 2);
    assert_eq!(exchanges[0], exchanges[1]);
    assert_eq!(exchanges[0].initiator_link_id, completed.id);
    assert_eq!(exchanges[0].initiator_subject, owner.to_string());
    assert_eq!(
        exchanges[0].initiator_display_name.as_deref(),
        Some("timeout-owner")
    );
    assert_eq!(
        service
            .complete(principal, start.request_id, code.clone(), Utc::now())
            .await?,
        completed
    );
    assert_eq!(scripted.exchanges.lock().expect("exchanges").len(), 2);
    let retained: (bool,bool,bool,bool) = sqlx::query_as("SELECT initiator_link_id IS NULL,secret_ciphertext IS NULL,secret_nonce IS NULL,secret_key_version IS NULL FROM suite_link_requests WHERE id=$1").bind(start.request_id).fetch_one(&db.pool).await?;
    assert_eq!(retained, (true, true, true, true));
    for invalid in 0..5 {
        let mut response = valid.clone();
        match invalid {
            0 => response.link_id = Uuid::nil(),
            1 => response.display_name = Some("é".repeat(129)),
            2 => response.display_name = Some("remote\nname".into()),
            3 => response
                .authorizer_sends
                .push("unknown.activity.completed".into()),
            _ => response.subject.clear(),
        }
        scripted
            .responses
            .lock()
            .expect("responses")
            .push_back(Ok(response));
        let owner = user(&db.pool, &format!("invalid-owner-{invalid}")).await?;
        let principal = SuiteUser {
            id: owner,
            ..SuiteUser::default()
        };
        let start = service
            .start(
                principal.clone(),
                LinkStartRequest {
                    peer_app: "beta".into(),
                    return_url: "alpha://suite/linked".into(),
                    state_nonce: "nonce".into(),
                },
                Utc::now(),
            )
            .await?;
        assert_eq!(
            service
                .complete(principal, start.request_id, code.clone(), Utc::now())
                .await
                .expect_err("invalid peer response")
                .code(),
            "suite_peer_unreachable"
        );
        let pending: (Option<chrono::DateTime<Utc>>, Option<Uuid>) =
            sqlx::query_as("SELECT consumed_at,link_id FROM suite_link_requests WHERE id=$1")
                .bind(start.request_id)
                .fetch_one(&db.pool)
                .await?;
        assert_eq!(pending, (None, None));
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_links WHERE user_id=$1")
            .bind(owner)
            .fetch_one(&db.pool)
            .await?;
        assert_eq!(rows, 0);
    }
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(jobs, 0);
    scripted
        .responses
        .lock()
        .expect("responses")
        .extend([Ok(valid.clone()), Ok(valid.clone())]);
    scripted.exchanges.lock().expect("exchanges").clear();
    *scripted.barrier.lock().expect("barrier") = Some(Arc::new(tokio::sync::Barrier::new(2)));
    let principal = SuiteUser {
        id: owner,
        ..SuiteUser::default()
    };
    let started = service
        .start(
            principal.clone(),
            LinkStartRequest {
                peer_app: "beta".into(),
                return_url: "alpha://suite/linked".into(),
                state_nonce: "nonce".into(),
            },
            Utc::now(),
        )
        .await?;
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        tokio::join!(
            service.complete(
                principal.clone(),
                started.request_id,
                code.clone(),
                Utc::now()
            ),
            service.complete(
                principal.clone(),
                started.request_id,
                code.clone(),
                Utc::now()
            ),
        )
    })
    .await
    .expect("both completes reach exchange without holding the transaction");
    let first = first?;
    assert_eq!(first, second?);
    let exchanges = scripted.exchanges.lock().expect("exchanges").clone();
    assert_eq!(exchanges.len(), 2);
    assert_eq!(exchanges[0], exchanges[1]);
    assert_eq!(exchanges[0].initiator_link_id, first.id);
    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM suite_links WHERE user_id=$1 AND status='active'")
            .bind(owner)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(active, 1);
    service.disconnect(owner, first.id, Utc::now()).await?;
    service.disconnect(owner, first.id, Utc::now()).await?;
    let revokes: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox WHERE payload->>'link_id'=$1 AND job_type='suite.links.revoke'")
        .bind(first.id.to_string()).fetch_one(&db.pool).await?;
    assert_eq!(revokes, 1);
    assert_eq!(
        service
            .complete(principal, started.request_id, code, Utc::now())
            .await
            .expect_err("revoked completed request")
            .code(),
        "not_found"
    );
    Ok(())
}

struct ReplayGrantApplier {
    ledger_only: bool,
}
#[async_trait]
impl SuiteEventApplier for ReplayGrantApplier {
    async fn apply(
        &self,
        connection: &mut sqlx::PgConnection,
        event: SuiteApplyEvent<'_>,
    ) -> Result<AppliedOutcome, SuiteStoreError> {
        assert!(event.replay);
        assert_eq!(event.reward_mode, RewardMode::Off);
        let mut outcome = Applier::default().apply(connection, event).await?;
        if self.ledger_only {
            outcome.ledger_entry_id = Some("invalid-replay-ledger".into());
        } else {
            outcome.outcome = AppliedOutcomeStatus::Granted;
        }
        Ok(outcome)
    }
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn a_replay_grant_or_ledger_entry_rolls_back_the_inbox_and_product_write() -> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    let mut event = activity(12, Some(3)).envelope("remote", "beta");
    event.event_type = "beta.activity.completed".into();
    event.occurred_at = Utc::now() - chrono::Duration::days(300);
    for ledger_only in [false, true] {
        let app = common::app_with_suite(
            &db.pool,
            module(
                &db.pool,
                context.clone(),
                Arc::new(ReplayGrantApplier { ledger_only }),
            ),
        );
        let response = signed_route(&app, link.id, &event, &secret(&link), true).await?;
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["error"]["code"], "internal_server_error");
        let rows: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM suite_inbound_events),(SELECT count(*) FROM applied_events)").fetch_one(&db.pool).await?;
        assert_eq!(rows, (0, 0));
        assert!(
            SuiteLinkService::new(context.clone())
                .get(owner, link.id)
                .await?
                .last_received_at
                .is_none()
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn exchange_and_complete_lock_the_owner_before_codes_and_requests_during_erasure()
-> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let service = Arc::new(SuiteLinkService::new(context.clone()));
    let verifier = URL_SAFE_NO_PAD.encode([9u8; 32]);
    let authorized = service
        .authorize(
            SuiteUser {
                id: owner,
                ..SuiteUser::default()
            },
            AuthorizationRequest {
                client: "beta".into(),
                state: verifier.clone(),
                code_challenge: pkce_challenge(&verifier)?,
                hint: None,
            },
            Utc::now(),
        )
        .await?;
    let input = ExchangeRequest {
        client: "beta".into(),
        code: query(&authorized.redirect_url, "code"),
        code_verifier: verifier.clone(),
        initiator_link_id: Uuid::now_v7(),
        link_secret: verifier,
        initiator_subject: "remote".into(),
        initiator_suite_subject: None,
        initiator_display_name: None,
    };
    let mut erasure = db.pool.begin().await?;
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *erasure)
        .await?;
    context.store.lock_ingest_user(&mut erasure, owner).await?;
    let exchange = tokio::spawn(async move { service.exchange(input, Utc::now()).await });
    wait_for_blocked_connection(&db.pool, blocker).await?;
    sqlx::query("SET LOCAL lock_timeout='1s'")
        .execute(&mut *erasure)
        .await?;
    PostgresSuiteLinkStore::new(db.pool.clone())
        .erase_owner(&mut erasure, owner)
        .await?;
    erasure.commit().await?;
    assert_eq!(
        exchange.await?.expect_err("erased code").code(),
        "suite_code_invalid"
    );
    let rows: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM suite_links),(SELECT count(*) FROM suite_link_codes)",
    )
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(rows, (0, 0));

    let service = SuiteLinkService::new(context.clone());
    let principal = SuiteUser {
        id: owner,
        ..SuiteUser::default()
    };
    let request = service
        .start(
            principal.clone(),
            LinkStartRequest {
                peer_app: "beta".into(),
                return_url: format!(
                    "{}/suite/linked",
                    context.registry.public_web_url().expect("web URL")
                ),
                state_nonce: URL_SAFE_NO_PAD.encode([1u8; 32]),
            },
            Utc::now(),
        )
        .await?;
    let mut erasure = db.pool.begin().await?;
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *erasure)
        .await?;
    context.store.lock_ingest_user(&mut erasure, owner).await?;
    let completion = tokio::spawn(async move {
        service
            .complete(
                principal,
                request.request_id,
                URL_SAFE_NO_PAD.encode([2u8; 32]),
                Utc::now(),
            )
            .await
    });
    wait_for_blocked_connection(&db.pool, blocker).await?;
    sqlx::query("SET LOCAL lock_timeout='1s'")
        .execute(&mut *erasure)
        .await?;
    PostgresSuiteLinkStore::new(db.pool.clone())
        .erase_owner(&mut erasure, owner)
        .await?;
    erasure.commit().await?;
    assert_eq!(
        completion.await?.expect_err("erased request").code(),
        "not_found"
    );
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_link_requests")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(requests, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn erasure_waits_for_product_writes_before_deleting_their_suite_jobs() -> TestResult {
    use baukit_erasure::ProductErasure;

    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    sqlx::query("CREATE TABLE product_facts(owner_id uuid REFERENCES users(id) ON DELETE CASCADE, sequence integer NOT NULL)")
        .execute(&db.pool).await?;
    let peer = Arc::new(ScriptedWebhookReceiver::start().await?);
    let adapter = PostgresSuiteErasure::new(
        PostgresSuiteLinkStore::new(db.pool.clone()),
        Arc::new(Identities(db.pool.clone())),
        Arc::new(ProductDeletion {
            peer,
            expected_calls: 0,
        }),
    );
    let mut domain = db.pool.begin().await?;
    context
        .outbox
        .lock_owner_in_transaction(&mut domain, owner)
        .await?;
    sqlx::query("INSERT INTO product_facts(owner_id,sequence) VALUES($1,1)")
        .bind(owner)
        .execute(&mut *domain)
        .await?;
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *domain)
        .await?;
    let pool = db.pool.clone();
    let erasure = tokio::spawn(async move {
        let mut transaction = pool.begin().await?;
        adapter.erase(&mut transaction, &owner.to_string()).await?;
        transaction.commit().await
    });
    wait_for_blocked_connection(&db.pool, blocker).await?;
    assert_eq!(
        context
            .outbox
            .enqueue_in_transaction(&mut domain, owner, &[activity(10, None)])
            .await?,
        1
    );
    domain.commit().await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), erasure).await???;
    let rows: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM product_facts WHERE owner_id=$1),
        (SELECT count(*) FROM suite_links WHERE user_id=$1),
        (SELECT count(*) FROM job_outbox WHERE payload->>'link_id'=$2)",
    )
    .bind(owner)
    .bind(link.id.to_string())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(rows, (0, 0, 0));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn link_job_routes_wait_for_erasure_before_locking_links() -> TestResult {
    let db = common::postgres_database().await?;
    let context = make_context(&db.pool, registry("alpha", "beta", 12340, 12341));
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context.clone(), Arc::new(Applier::default())),
    );
    for operation in ["test", "replay", "disconnect"] {
        let owner = user(&db.pool, operation).await?;
        let (link, _) =
            exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
        sqlx::query("UPDATE suite_links SET last_replay_at=NULL WHERE id=$1")
            .bind(link.id)
            .execute(&db.pool)
            .await?;
        let mut erasure = db.pool.begin().await?;
        context.store.lock_ingest_user(&mut erasure, owner).await?;
        let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *erasure)
            .await?;
        let (method, path) = if operation == "disconnect" {
            ("DELETE", format!("/suite/links/{}", link.id))
        } else {
            ("POST", format!("/suite/links/{}/{operation}", link.id))
        };
        let server = app.clone();
        let request = tokio::spawn(async move {
            call(
                &server,
                method,
                &path,
                Some(&owner.to_string()),
                json!({"since":Utc::now().date_naive()}),
            )
            .await
            .map_err(|error| error.to_string())
        });
        wait_for_blocked_connection(&db.pool, blocker).await?;
        PostgresSuiteLinkStore::new(db.pool.clone())
            .erase_owner(&mut erasure, owner)
            .await?;
        erasure.commit().await?;
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), request)
            .await??
            .map_err(std::io::Error::other)?;
        assert_eq!(response.0, StatusCode::NOT_FOUND, "{operation}");
        assert_eq!(response.1["error"]["code"], "not_found");
        let jobs: i64 =
            sqlx::query_scalar("SELECT count(*) FROM job_outbox WHERE payload->>'link_id'=$1")
                .bind(link.id.to_string())
                .fetch_one(&db.pool)
                .await?;
        assert_eq!(jobs, 0, "{operation}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn unfenced_product_inserts_and_updates_cannot_enqueue_during_erasure() -> TestResult {
    use baukit_erasure::ProductErasure;

    let db = common::postgres_database().await?;
    sqlx::query("CREATE TABLE product_facts(owner_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE, sequence integer NOT NULL)")
        .execute(&db.pool).await?;
    let context = make_context(&db.pool, registry("alpha", "beta", 12340, 12341));
    let peer = Arc::new(ScriptedWebhookReceiver::start().await?);
    for operation in ["insert", "update"] {
        let owner = user(&db.pool, &format!("unfenced-{operation}")).await?;
        let (link, _) =
            exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
        if operation == "update" {
            sqlx::query("INSERT INTO product_facts(owner_id,sequence) VALUES($1,1)")
                .bind(owner)
                .execute(&db.pool)
                .await?;
        }
        let mut domain = db.pool.begin().await?;
        let statement = if operation == "insert" {
            "INSERT INTO product_facts(owner_id,sequence) VALUES($1,1)"
        } else {
            "UPDATE product_facts SET sequence=2 WHERE owner_id=$1"
        };
        sqlx::query(statement)
            .bind(owner)
            .execute(&mut *domain)
            .await?;
        let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *domain)
            .await?;
        let adapter = PostgresSuiteErasure::new(
            PostgresSuiteLinkStore::new(db.pool.clone()),
            Arc::new(Identities(db.pool.clone())),
            Arc::new(ProductDeletion {
                peer: peer.clone(),
                expected_calls: 0,
            }),
        );
        let pool = db.pool.clone();
        let erasure = tokio::spawn(async move {
            let mut transaction = pool.begin().await?;
            adapter.erase(&mut transaction, &owner.to_string()).await?;
            transaction.commit().await
        });
        wait_for_blocked_connection(&db.pool, blocker).await?;
        assert_eq!(
            context
                .outbox
                .enqueue_in_transaction(&mut domain, owner, &[activity(10, None)])
                .await
                .expect_err("erasure owns the suite lock"),
            SuiteStoreError::Timeout,
            "{operation}"
        );
        domain.commit().await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), erasure).await???;
        let rows: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM product_facts WHERE owner_id=$1),
            (SELECT count(*) FROM suite_links WHERE user_id=$1),
            (SELECT count(*) FROM job_outbox WHERE payload->>'link_id'=$2)",
        )
        .bind(owner)
        .bind(link.id.to_string())
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(rows, (0, 0, 0), "{operation}");
    }
    Ok(())
}

async fn wait_for_blocked_connection(pool: &PgPool, blocker: i32) -> TestResult {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<(), sqlx::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn two_cleanups_delivery_failure_and_erasure_do_not_invert_link_and_job_locks() -> TestResult
{
    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    let now = Utc::now();
    sqlx::query("UPDATE suite_links SET created_at=$2 WHERE id=$1")
        .bind(link.id)
        .bind(now - chrono::Duration::days(8))
        .execute(&db.pool)
        .await?;
    let job = PostgresJobStore::new(db.pool.clone())
        .enqueue(NewJob::new(
            SUITE_EVENTS_DELIVER_JOB_TYPE,
            json!({"link_id":link.id}),
            1,
        ))
        .await?
        .job;
    let mut failure = db.pool.begin().await?;
    let failure_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *failure)
        .await?;
    sqlx::query("SELECT id FROM job_outbox WHERE id=$1 FOR UPDATE")
        .bind(job.id)
        .execute(&mut *failure)
        .await?;
    let store = PostgresSuiteLinkStore::new(db.pool.clone());
    let cleanup_store = store.clone();
    let cleanup = tokio::spawn(async move { cleanup_store.cleanup(now).await });
    wait_for_blocked_connection(&db.pool, failure_pid).await?;
    let cleanup_pid: i32 =
        sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid))")
            .bind(failure_pid)
            .fetch_one(&db.pool)
            .await?;
    let pool = db.pool.clone();
    let erasure_store = store.clone();
    let erasure = tokio::spawn(async move {
        let mut tx = pool.begin().await?;
        erasure_store.erase_owner(&mut tx, owner).await?;
        tx.commit().await
    });
    wait_for_blocked_connection(&db.pool, cleanup_pid).await?;
    let second =
        tokio::time::timeout(std::time::Duration::from_secs(5), store.cleanup(now)).await??;
    assert_eq!(second, SuiteCleanupOutcome::default());
    tokio::time::timeout(std::time::Duration::from_secs(5),
        sqlx::query("UPDATE job_outbox SET status='failed',failure_reason='permanent',last_error='suite_rejected' WHERE id=$1")
            .bind(job.id).execute(&mut *failure),
    ).await??;
    failure.commit().await?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), cleanup).await???;
    assert_eq!(result.unused_links, 1);
    tokio::time::timeout(std::time::Duration::from_secs(5), erasure).await???;
    let rows: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM suite_links),(SELECT count(*) FROM job_outbox),
        (SELECT count(*) FROM suite_failed_delivery_jobs),(SELECT count(*) FROM suite_link_codes),
        (SELECT count(*) FROM suite_link_requests)",
    )
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(rows, (0, 0, 0, 0, 0));
    Ok(())
}

struct OwnerRowApplier;
#[async_trait]
impl SuiteEventApplier for OwnerRowApplier {
    async fn lock_owner(
        &self,
        tx: &mut sqlx::PgConnection,
        owner: Uuid,
    ) -> Result<(), SuiteStoreError> {
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
            .bind(owner)
            .execute(tx)
            .await
            .map_err(|error| SuiteStoreError::Storage(error.to_string()))?;
        Ok(())
    }
    async fn apply(
        &self,
        tx: &mut sqlx::PgConnection,
        event: SuiteApplyEvent<'_>,
    ) -> Result<AppliedOutcome, SuiteStoreError> {
        Applier::default().apply(tx, event).await
    }
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn ingest_locks_the_product_owner_before_the_link_and_applies_in_that_transaction()
-> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    let mut product = db.pool.begin().await?;
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *product)
        .await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(owner)
        .execute(&mut *product)
        .await?;
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context, Arc::new(OwnerRowApplier)),
    );
    let mut event = activity(5, None).envelope("remote", "beta");
    event.event_type = "beta.activity.completed".into();
    let key = secret(&link);
    let ingest = tokio::spawn(async move {
        signed_route(&app, link.id, &event, &key, false)
            .await
            .map_err(|error| error.to_string())
    });
    wait_for_blocked_connection(&db.pool, blocker).await?;
    let mut probe = db.pool.begin().await?;
    sqlx::query("SELECT id FROM suite_links WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(link.id)
        .execute(&mut *probe)
        .await?;
    probe.rollback().await?;
    sqlx::query("UPDATE users SET display_name='after product write' WHERE id=$1")
        .bind(owner)
        .execute(&mut *product)
        .await?;
    product.commit().await?;
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), ingest)
        .await??
        .map_err(std::io::Error::other)?;
    assert_eq!(response.0, StatusCode::ACCEPTED);
    let writes: i64 = sqlx::query_scalar("SELECT count(*) FROM applied_events WHERE owner_id=$1")
        .bind(owner)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(writes, 1);
    Ok(())
}

struct TypedPayloadApplier;
#[async_trait]
impl SuiteEventApplier for TypedPayloadApplier {
    async fn apply(
        &self,
        tx: &mut sqlx::PgConnection,
        event: SuiteApplyEvent<'_>,
    ) -> Result<AppliedOutcome, SuiteStoreError> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Activity {
            activity_id: Uuid,
            #[serde(
                deserialize_with = "baukit_suite::domain::validation::integer::<_, u32, 1, 10>"
            )]
            minutes: u32,
        }
        let outcome = Applier::default().apply(tx, event).await?;
        let typed: Activity = event.payload.deserialize()?;
        assert!(!typed.activity_id.is_nil());
        assert!((1..=10).contains(&typed.minutes));
        Ok(outcome)
    }
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn typed_payload_rejection_returns_422_and_rolls_back_all_writes() -> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let (link, _) =
        exchange_link(&SuiteLinkService::new(context.clone()), owner, None, None).await?;
    let app = common::app_with_suite(
        &db.pool,
        module(&db.pool, context.clone(), Arc::new(TypedPayloadApplier)),
    );
    let mut event = activity(24, None).envelope("remote", "beta");
    event.event_type = "beta.activity.completed".into();
    let response = signed_route(&app, link.id, &event, &secret(&link), false).await?;
    assert_eq!(response.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.1["error"]["code"], "suite_payload_invalid");
    assert!(!response.2.contains_key("retry-after"));
    let rows: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM suite_inbound_events),(SELECT count(*) FROM applied_events)",
    )
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(rows, (0, 0));
    assert!(
        context
            .store
            .find_link(link.id)
            .await?
            .expect("link")
            .last_received_at
            .is_none()
    );
    event.payload.insert("minutes".into(), json!(5));
    assert_eq!(
        signed_route(&app, link.id, &event, &secret(&link), false)
            .await?
            .0,
        StatusCode::ACCEPTED
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn delivery_maps_401_and_410_then_stops_calling_a_revoked_peer() -> TestResult {
    let db = common::postgres_database().await?;
    let receiver = ScriptedWebhookReceiver::start().await?;
    receiver.push_response(ScriptedWebhookResponse::new(401));
    receiver.push_response(ScriptedWebhookResponse::new(410));
    let port = Url::parse(&receiver.url("/"))?.port().expect("port");
    let context = make_context(&db.pool, registry("alpha", "beta", 12340, port));
    let owner = user(&db.pool, "delivery-revocation").await?;
    let links = SuiteLinkService::new(context.clone());
    let (link, _) = exchange_link(&links, owner, None, None).await?;
    let job = SuiteDeliverJobV1::new(
        link.id,
        activity(5, None).envelope(&owner.to_string(), "alpha"),
        false,
    );
    let delivery = SuiteDeliveryService::new(context);
    assert_eq!(
        delivery.deliver(job.clone(), Utc::now()).await?,
        DeliveryAction::Unauthorized
    );
    let attention = links.get(owner, link.id).await?;
    assert_eq!(attention.status, LinkStatus::NeedsAttention);
    assert_eq!(attention.delivery_health, DeliveryHealth::NeedsAttention);
    assert_eq!(
        attention.last_failure_code.as_deref(),
        Some("suite_unauthorized")
    );
    assert_eq!(
        delivery.deliver(job.clone(), Utc::now()).await?,
        DeliveryAction::Revoked
    );
    assert_eq!(links.get(owner, link.id).await?.status, LinkStatus::Revoked);
    assert_eq!(receiver.calls(), 2);
    assert_eq!(
        delivery.deliver(job, Utc::now()).await?,
        DeliveryAction::LinkRevoked
    );
    assert_eq!(receiver.calls(), 2);
    Ok(())
}

async fn issue_exchange_request(
    service: &SuiteLinkService,
    owner: Uuid,
    client: &str,
    now: chrono::DateTime<Utc>,
) -> Result<ExchangeRequest, Box<dyn Error>> {
    let verifier = URL_SAFE_NO_PAD.encode([9_u8; 32]);
    let authorized = service
        .authorize(
            SuiteUser {
                id: owner,
                ..SuiteUser::default()
            },
            AuthorizationRequest {
                client: client.into(),
                state: verifier.clone(),
                code_challenge: pkce_challenge(&verifier)?,
                hint: None,
            },
            now,
        )
        .await?;
    Ok(ExchangeRequest {
        client: client.into(),
        code: query(&authorized.redirect_url, "code"),
        code_verifier: verifier.clone(),
        initiator_link_id: Uuid::now_v7(),
        link_secret: verifier,
        initiator_subject: "remote".into(),
        initiator_suite_subject: None,
        initiator_display_name: None,
    })
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn failed_exchange_budget_counts_every_invalid_code_and_allows_a_fresh_valid_code()
-> TestResult {
    let (db, mut context, owner) = authorizer_fixture().await?;
    let limiter = Arc::new(FixedWindowLimiter::default());
    Arc::get_mut(&mut context).expect("context").limiter = limiter.clone();
    let service = SuiteLinkService::new(context);
    let now = Utc::now();
    let valid = issue_exchange_request(&service, owner, "beta", now).await?;
    let mut unknown = valid.clone();
    unknown.code = URL_SAFE_NO_PAD.encode([8_u8; 32]);
    let expired =
        issue_exchange_request(&service, owner, "beta", now - chrono::Duration::seconds(61))
            .await?;
    let mut wrong_client = issue_exchange_request(&service, owner, "hebkit", now).await?;
    wrong_client.client = "beta".into();
    let mut wrong_verifier = valid;
    wrong_verifier.code_verifier = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let invalid = [unknown, expired, wrong_client, wrong_verifier];
    for index in 0..SUITE_EXCHANGE_FAILURE_LIMIT {
        let request = invalid[usize::try_from(index)? % invalid.len()].clone();
        assert_eq!(
            service
                .exchange(request, now)
                .await
                .expect_err("invalid code")
                .code(),
            "suite_code_invalid"
        );
        assert_eq!(
            limiter.counts.lock().expect("counts")["suite:exchange:beta"],
            u64::from(index + 1)
        );
    }
    for request in &invalid {
        assert!(matches!(
            service.exchange(request.clone(), now).await,
            Err(SuiteServiceError::RateLimited(60))
        ));
    }
    let fresh = issue_exchange_request(&service, owner, "beta", now).await?;
    let completed = service.exchange(fresh, now).await?;
    assert_eq!(
        service.get(owner, completed.link_id).await?.status,
        LinkStatus::Active
    );
    assert_eq!(
        limiter.counts.lock().expect("counts")["suite:exchange:beta"],
        20
    );
    assert_eq!(limiter.calls.lock().expect("calls").len(), 24);
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM suite_links")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(links, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn lock_order_migration_preserves_old_failures_and_accounts_new_jobs_once() -> TestResult {
    let db = common::postgres_database_before_lock_order().await?;
    let owner = user(&db.pool, "suite-upgrade").await?;
    let link = Uuid::now_v7();
    sqlx::query("INSERT INTO suite_links(id,user_id,peer_app,role,remote_link_id,remote_subject,secret_ciphertext,secret_nonce,secret_key_version,sends,receives) VALUES($1,$2,'beta','authorizer',$3,'remote',$4,$5,1,'{}','{}')")
        .bind(link).bind(owner).bind(Uuid::now_v7()).bind(vec![1u8]).bind(vec![2u8])
        .execute(&db.pool).await?;
    let jobs = PostgresJobStore::new(db.pool.clone());
    let old_job = jobs
        .enqueue(NewJob::new(
            SUITE_EVENTS_DELIVER_JOB_TYPE,
            json!({"link_id":link}),
            1,
        ))
        .await?
        .job;
    sqlx::query("UPDATE job_outbox SET status='failed',failure_reason='permanent',last_error='suite_rejected' WHERE id=$1")
        .bind(old_job.id).execute(&db.pool).await?;
    let old_count: i32 =
        sqlx::query_scalar("SELECT consecutive_failures FROM suite_links WHERE id=$1")
            .bind(link)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(old_count, 1);

    sqlx::raw_sql(SUITE_LOCK_ORDER_MIGRATION_SQL)
        .execute(&db.pool)
        .await?;
    let store = PostgresSuiteLinkStore::new(db.pool.clone());
    assert_eq!(
        store
            .find_link(link)
            .await?
            .expect("link")
            .consecutive_failures,
        1
    );
    let old_accounted: bool =
        sqlx::query_scalar("SELECT accounted FROM suite_failed_delivery_jobs WHERE job_id=$1")
            .bind(old_job.id)
            .fetch_one(&db.pool)
            .await?;
    assert!(old_accounted);

    let new_job = jobs
        .enqueue(NewJob::new(
            SUITE_EVENTS_DELIVER_JOB_TYPE,
            json!({"link_id":link}),
            1,
        ))
        .await?
        .job;
    sqlx::query("UPDATE job_outbox SET status='failed',failure_reason='permanent',last_error='suite_unauthorized' WHERE id=$1")
        .bind(new_job.id).execute(&db.pool).await?;
    let pending: bool =
        sqlx::query_scalar("SELECT NOT accounted FROM suite_failed_delivery_jobs WHERE job_id=$1")
            .bind(new_job.id)
            .fetch_one(&db.pool)
            .await?;
    assert!(pending);
    let counted = store.find_link(link).await?.expect("link");
    assert_eq!(counted.consecutive_failures, 2);
    assert_eq!(
        counted.last_failure_code.as_deref(),
        Some("suite_unauthorized")
    );
    sqlx::query("UPDATE job_outbox SET status='pending',failure_reason=NULL WHERE id=$1")
        .bind(new_job.id)
        .execute(&db.pool)
        .await?;
    sqlx::query("UPDATE job_outbox SET status='failed',failure_reason='permanent' WHERE id=$1")
        .bind(new_job.id)
        .execute(&db.pool)
        .await?;
    assert_eq!(
        store
            .find_link(link)
            .await?
            .expect("link")
            .consecutive_failures,
        2
    );
    let pending_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM suite_failed_delivery_jobs WHERE NOT accounted")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(pending_count, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn cleanup_does_not_wait_for_an_owner_without_cleanup_work() -> TestResult {
    let (db, context, owner) = authorizer_fixture().await?;
    let service = SuiteLinkService::new(context.clone());
    let (expired, _) = exchange_link(&service, owner, None, None).await?;
    let recent_owner = user(&db.pool, "recent-owner").await?;
    let (recent, _) = exchange_link(&service, recent_owner, None, None).await?;
    let now = Utc::now();
    sqlx::query("UPDATE suite_links SET created_at=$2 WHERE id=$1")
        .bind(expired.id)
        .bind(now - chrono::Duration::days(8))
        .execute(&db.pool)
        .await?;
    let mut busy = db.pool.begin().await?;
    context
        .store
        .lock_ingest_user(&mut busy, recent_owner)
        .await?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        context.store.cleanup(now),
    )
    .await??;
    assert_eq!(result.unused_links, 1);
    assert!(context.store.find_link(expired.id).await?.is_none());
    assert_eq!(
        context
            .store
            .find_link(recent.id)
            .await?
            .expect("recent link")
            .status,
        LinkStatus::Active
    );
    busy.rollback().await?;
    Ok(())
}
