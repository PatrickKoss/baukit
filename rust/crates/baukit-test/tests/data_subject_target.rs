#![cfg(feature = "data-subject")]

use std::time::Duration;

use aws_lc_rs::digest::{SHA256, digest};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_data_subject::{
    CategoryAction, CategoryEntry, DiscoveryDocument, IdentifierType, OperationResource,
    OperationStatus, ReceiptError, ReceiptKeyStatus, ReceiptPayload, ReceiptVerifier, Restriction,
};
use baukit_test::{
    DataSubjectFault as Fault, DataSubjectOutcome as Outcome, DataSubjectScript as Script,
    DataSubjectScriptError, DataSubjectTargetOptions, DataSubjectTargetServer as Server,
};
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

const SUBJECT: &str = "client-subject-reference-01";
const KEY: &str = "idempotency-key-0001";
const TOKEN: &str = "test-bearer-credential";
const ERASE: &str = "/api/v1/data-subject/erasures";
const EXPORT: &str = "/api/v1/data-subject/exports";

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("test client")
}

async fn target() -> Result<Server> {
    let server = Server::start().await?;
    server.register_token(
        TOKEN,
        "privacy-client",
        ["data-subject.erase", "data-subject.export"],
    );
    Ok(server)
}

fn body(subject: &str) -> Value {
    json!({"subjectReference": subject, "identifiers": [{"type": "email", "value": "Fixture@Example.invalid"}]})
}

async fn post(client: &Client, server: &Server, path: &str, subject: &str) -> Result<Response> {
    Ok(client
        .post(format!("{}{path}", server.base_url()))
        .bearer_auth(TOKEN)
        .header("idempotency-key", KEY)
        .json(&body(subject))
        .send()
        .await?)
}

async fn start(
    client: &Client,
    server: &Server,
    path: &str,
    subject: &str,
) -> Result<OperationResource> {
    let response = post(client, server, path, subject).await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation: OperationResource = response.json().await?;
    assert_eq!(operation.status, OperationStatus::Pending);
    assert!(
        operation.receipt.is_none()
            && operation.completed_at.is_none()
            && operation.error.is_none()
    );
    Ok(operation)
}

fn operation_url(server: &Server, path: &str, operation: &OperationResource) -> String {
    format!("{}{path}/{}", server.base_url(), operation.operation_id)
}

async fn poll(client: &Client, url: &str) -> Result<OperationResource> {
    let response = client.get(url).bearer_auth(TOKEN).send().await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(response.json().await?)
}

fn verify(server: &Server, operation: &OperationResource) -> Result<ReceiptPayload> {
    let payload = ReceiptVerifier::from_discovery(&server.discovery())?
        .verify(operation.receipt.as_ref().expect("terminal receipt"))?;
    assert_eq!(payload.operation_id, operation.operation_id);
    assert_eq!(payload.kind, operation.kind);
    assert_eq!(payload.status, operation.status);
    assert!(payload.subject_reference == operation.subject_reference);
    assert_eq!(payload.service, server.discovery().service);
    assert_eq!(payload.created_at, operation.created_at);
    assert_eq!(Some(payload.completed_at), operation.completed_at);
    assert_eq!(payload.categories, operation.categories);
    assert_eq!(payload.restrictions, operation.restrictions);
    assert_eq!(payload.parts, operation.parts);
    assert!(
        !String::from_utf8(
            STANDARD.decode(&operation.receipt.as_ref().expect("receipt").payload)?
        )?
        .to_lowercase()
        .contains("fixture@example.invalid")
    );
    Ok(payload)
}

async fn error(response: Response, status: StatusCode, code: &str) -> Result {
    assert_eq!(response.status(), status);
    let value: Value = response.json().await?;
    assert_eq!(value["error"]["code"], code);
    assert!(value["error"]["message"].is_string());
    assert!(
        value["error"]["requestId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert_eq!(value["error"]["details"], json!({}));
    assert!(!value.to_string().contains(TOKEN));
    assert!(!value.to_string().contains("Fixture@Example.invalid"));
    Ok(())
}

fn categories() -> Vec<CategoryEntry> {
    vec![CategoryEntry {
        category: "records".into(),
        action: CategoryAction::Erased,
        count: 3,
    }]
}

#[tokio::test]
async fn discovery_and_every_terminal_outcome_have_contract_receipts() -> Result {
    let server = target().await?;
    let client = client();
    let response = client
        .get(format!(
            "{}/.well-known/baukit/data-subject",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "public, max-age=300");
    let discovery: DiscoveryDocument = response.json().await?;
    assert_eq!(discovery, server.discovery());
    assert_eq!(discovery.receipt_keys, vec![server.receipt_key()]);
    ReceiptVerifier::from_discovery(&discovery)?;
    let restrictions = vec![Restriction {
        category: "invoices".into(),
        reason: "Statutory retention".into(),
        legal_basis: "Tax law".into(),
        retain_until: Some("2030-01-01".parse()?),
    }];
    for (index, (outcome, expected)) in [
        (
            Outcome::Completed {
                categories: categories(),
            },
            OperationStatus::Completed,
        ),
        (Outcome::NotFound, OperationStatus::NotFound),
        (
            Outcome::Restricted {
                categories: vec![CategoryEntry {
                    category: "invoices".into(),
                    action: CategoryAction::Retained,
                    count: 2,
                }],
                restrictions: restrictions.clone(),
            },
            OperationStatus::Restricted,
        ),
        (
            Outcome::Failed {
                code: "product_unavailable".into(),
            },
            OperationStatus::Failed,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let reference = format!("subject-reference-{index:04}");
        server.set_script(&reference, outcome)?;
        let pending = start(&client, &server, ERASE, &reference).await?;
        let url = operation_url(&server, ERASE, &pending);
        let terminal = poll(&client, &url).await?;
        assert_eq!(terminal.status, expected);
        assert!(terminal.completed_at.is_some());
        if expected == OperationStatus::Failed {
            assert!(terminal.receipt.is_none());
            assert_eq!(
                terminal.error.as_ref().expect("failure").code,
                "product_unavailable"
            );
        } else {
            verify(&server, &terminal)?;
            assert!(terminal.error.is_none());
        }
        if expected == OperationStatus::NotFound {
            assert!(
                terminal.categories.is_empty()
                    && terminal.restrictions.is_empty()
                    && terminal.parts.is_empty()
            );
        }
        if expected == OperationStatus::Restricted {
            assert_eq!(terminal.restrictions, restrictions);
            assert_eq!(
                terminal.categories,
                vec![CategoryEntry {
                    category: "invoices".into(),
                    action: CategoryAction::Retained,
                    count: 2,
                }]
            );
        }
        if expected == OperationStatus::Completed {
            assert_eq!(terminal.categories, categories());
        }
        assert_eq!(poll(&client, &url).await?, terminal);
        let replay = post(&client, &server, ERASE, &reference).await?;
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(replay.json::<OperationResource>().await?, terminal);
        let conflict = client
            .post(format!("{}{ERASE}", server.base_url()))
            .bearer_auth(TOKEN)
            .header("idempotency-key", KEY)
            .json(&body("different-reference-01"))
            .send()
            .await?;
        error(
            conflict,
            StatusCode::CONFLICT,
            "data_subject_idempotency_conflict",
        )
        .await?;
        // Each iteration uses an independent client registration to isolate the same key.
        server.register_token(
            TOKEN,
            format!("privacy-client-{index}"),
            ["data-subject.erase", "data-subject.export"],
        );
    }
    Ok(())
}

#[tokio::test]
async fn pending_polls_normalized_replay_and_concurrent_posts_preserve_one_operation() -> Result {
    let server = target().await?;
    let client = client();
    server.set_default_script(Outcome::NotFound)?;
    server.set_script(
        SUBJECT,
        Outcome::PendingFor {
            polls: 2,
            then: Box::new(Outcome::PendingFor {
                polls: 1,
                then: Box::new(Outcome::Completed {
                    categories: categories(),
                }),
            }),
        },
    )?;
    let (a, b) = tokio::join!(
        post(&client, &server, ERASE, SUBJECT),
        post(&client, &server, ERASE, SUBJECT)
    );
    let a: OperationResource = a?.json().await?;
    let b: OperationResource = b?.json().await?;
    assert_eq!(a, b);
    let url = operation_url(&server, ERASE, &a);
    for _ in 0..3 {
        let replay = client
            .post(format!("{}{ERASE}", server.base_url()))
            .bearer_auth(TOKEN)
            .header("idempotency-key", KEY)
            .json(
                &json!({"identifiers": [{"value": " fixture@example.INVALID ", "type": "email"}],
                "subjectReference": SUBJECT}),
            )
            .send()
            .await?;
        assert_eq!(replay.status(), StatusCode::ACCEPTED);
        assert_eq!(replay.json::<OperationResource>().await?, a);
        assert_eq!(poll(&client, &url).await?.status, OperationStatus::Pending);
    }
    let done = poll(&client, &url).await?;
    assert_eq!(done.status, OperationStatus::Completed);
    verify(&server, &done)?;
    let fallback = start(&client, &server, EXPORT, "fallback-reference-01").await?;
    let fallback = poll(&client, &operation_url(&server, EXPORT, &fallback)).await?;
    assert_eq!(fallback.status, OperationStatus::NotFound);
    verify(&server, &fallback)?;
    let conflict = client
        .post(format!("{}{ERASE}", server.base_url()))
        .bearer_auth(TOKEN)
        .header("idempotency-key", KEY)
        .json(&json!({"subjectReference": SUBJECT,
            "identifiers": [{"type": "email", "value": "other@example.invalid"}]}))
        .send()
        .await?;
    error(
        conflict,
        StatusCode::CONFLICT,
        "data_subject_idempotency_conflict",
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn auth_ownership_scope_and_operation_kind_apply_to_all_routes() -> Result {
    let server = target().await?;
    let client = client();
    server.register_token(
        "same-client",
        "privacy-client",
        ["data-subject.erase", "data-subject.export"],
    );
    server.register_token(
        "other-client",
        "another-client",
        ["data-subject.erase", "data-subject.export"],
    );
    server.register_token("erase-only", "privacy-client", ["data-subject.erase"]);
    server.register_token("no-scope", "privacy-client", Vec::<String>::new());
    server.register_token(
        "no-client",
        "",
        ["data-subject.erase", "data-subject.export"],
    );
    for path in [ERASE, EXPORT] {
        let pending = start(&client, &server, path, SUBJECT).await?;
        let url = operation_url(&server, path, &pending);
        let mut routes = vec![
            (Method::POST, format!("{}{path}", server.base_url())),
            (Method::GET, url.clone()),
        ];
        if path == EXPORT {
            routes.push((Method::GET, format!("{url}/parts/1")));
        }
        for (method, route) in routes {
            for token in [None, Some("unknown-credential")] {
                let mut request = client
                    .request(method.clone(), &route)
                    .header("idempotency-key", KEY)
                    .json(&body(SUBJECT));
                if let Some(token) = token {
                    request = request.bearer_auth(token);
                }
                error(
                    request.send().await?,
                    StatusCode::UNAUTHORIZED,
                    "unauthenticated",
                )
                .await?;
            }
            for (token, code) in [
                ("no-scope", "insufficient_scope"),
                ("no-client", "data_subject_client_not_allowed"),
            ] {
                error(
                    client
                        .request(method.clone(), &route)
                        .bearer_auth(token)
                        .header("idempotency-key", KEY)
                        .json(&body(SUBJECT))
                        .send()
                        .await?,
                    StatusCode::FORBIDDEN,
                    code,
                )
                .await?;
            }
            if method == Method::GET {
                error(
                    client
                        .get(&route)
                        .bearer_auth("other-client")
                        .send()
                        .await?,
                    StatusCode::NOT_FOUND,
                    "data_subject_operation_not_found",
                )
                .await?;
            }
            if path == EXPORT {
                error(
                    client
                        .request(method, &route)
                        .bearer_auth("erase-only")
                        .header("idempotency-key", KEY)
                        .json(&body(SUBJECT))
                        .send()
                        .await?,
                    StatusCode::FORBIDDEN,
                    "insufficient_scope",
                )
                .await?;
            }
        }
        let same: OperationResource = client
            .get(&url)
            .bearer_auth("same-client")
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(same.operation_id, pending.operation_id);
        let other_kind = if path == ERASE { EXPORT } else { ERASE };
        error(
            client
                .get(operation_url(&server, other_kind, &pending))
                .bearer_auth(TOKEN)
                .send()
                .await?,
            StatusCode::NOT_FOUND,
            "data_subject_operation_not_found",
        )
        .await?;
        let independent: OperationResource = client
            .post(format!("{}{path}", server.base_url()))
            .bearer_auth("other-client")
            .header("idempotency-key", KEY)
            .json(&body(SUBJECT))
            .send()
            .await?
            .json()
            .await?;
        assert_ne!(independent.operation_id, pending.operation_id);
    }
    Ok(())
}

#[tokio::test]
async fn invalid_requests_use_safe_contract_errors() -> Result {
    let options = DataSubjectTargetOptions {
        identifier_types: vec![IdentifierType::Email, IdentifierType::Phone],
        limits: baukit_data_subject::Limits {
            max_identifiers: 2,
            max_identifier_length: 32,
            max_export_part_bytes: 16,
        },
        ..Default::default()
    };
    let server = Server::start_with_options(options).await?;
    server.register_token(TOKEN, "privacy-client", ["data-subject.erase"]);
    let client = client();
    let url = format!("{}{ERASE}", server.base_url());
    for key in [None, Some("short"), Some("invalid key with spaces")] {
        let mut request = client.post(&url).bearer_auth(TOKEN).json(&body(SUBJECT));
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        error(
            request.send().await?,
            StatusCode::BAD_REQUEST,
            "data_subject_idempotency_key_invalid",
        )
        .await?;
    }
    error(
        client
            .post(&url)
            .bearer_auth(TOKEN)
            .header("idempotency-key", KEY)
            .header("idempotency-key", KEY)
            .json(&body(SUBJECT))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "data_subject_idempotency_key_invalid",
    )
    .await?;
    let mut invalid = vec![
        json!({}),
        json!({"subjectReference": "short", "identifiers": []}),
        json!({"subjectReference": SUBJECT, "identifiers": []}),
    ];
    for (kind, value) in [
        ("email", " ".into()),
        ("email", "x".repeat(33)),
        ("phone", "+12".into()),
        ("phone", "+1234567x".into()),
        ("x-Bad", "value".into()),
    ] {
        invalid.push(
            json!({"subjectReference": SUBJECT, "identifiers": [{"type": kind, "value": value}]}),
        );
    }
    invalid.push(json!({"subjectReference": SUBJECT, "identifiers": vec![json!({"type": "email", "value": "a"}); 3]}));
    for body in invalid {
        error(
            client
                .post(&url)
                .bearer_auth(TOKEN)
                .header("idempotency-key", KEY)
                .json(&body)
                .send()
                .await?,
            StatusCode::UNPROCESSABLE_ENTITY,
            "data_subject_request_invalid",
        )
        .await?;
    }
    error(
        client
            .post(&url)
            .bearer_auth(TOKEN)
            .header("idempotency-key", KEY)
            .body("{broken")
            .send()
            .await?,
        StatusCode::UNPROCESSABLE_ENTITY,
        "data_subject_request_invalid",
    )
    .await?;
    for kind in ["accountId", "x-custom"] {
        error(client.post(&url).bearer_auth(TOKEN).header("idempotency-key", KEY)
            .json(&json!({"subjectReference": SUBJECT, "identifiers": [{"type": kind, "value": "value"}]})).send().await?,
            StatusCode::UNPROCESSABLE_ENTITY, "data_subject_identifier_type_unsupported").await?;
    }
    let oversized = server.set_script(
        SUBJECT,
        Outcome::Export {
            parts: vec![json!("x".repeat(20))],
        },
    );
    assert_eq!(oversized, Err(DataSubjectScriptError::ExportBounds));
    assert_eq!(
        server.set_script("short", Outcome::NotFound),
        Err(DataSubjectScriptError::SubjectReference)
    );
    assert_eq!(
        server.set_default_script(
            Script::new(Outcome::NotFound).with_fault(Fault::ServerError {
                status: 400,
                times: 1
            })
        ),
        Err(DataSubjectScriptError::ServerStatus)
    );
    Ok(())
}

#[tokio::test]
async fn exports_sign_exact_parts_and_enforce_readiness_expiry_and_retention() -> Result {
    let server = target().await?;
    let client = client();
    let expected_parts = vec![json!({"records": [1, 2]}), json!(["second", "part"])];
    server.set_script(
        SUBJECT,
        Outcome::Export {
            parts: expected_parts.clone(),
        },
    )?;
    let pending = start(&client, &server, EXPORT, SUBJECT).await?;
    let url = operation_url(&server, EXPORT, &pending);
    error(
        client
            .get(format!("{url}/parts/1"))
            .bearer_auth(TOKEN)
            .send()
            .await?,
        StatusCode::CONFLICT,
        "data_subject_export_not_ready",
    )
    .await?;
    let done = poll(&client, &url).await?;
    let payload = verify(&server, &done)?;
    assert_eq!(payload.parts.len(), 2);
    for part in &payload.parts {
        let response = client
            .get(format!("{url}/parts/{}", part.part_number))
            .bearer_auth(TOKEN)
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = response.bytes().await?;
        assert_eq!(bytes.len() as u64, part.bytes);
        assert_eq!(sha256(&bytes), part.sha256);
        assert!(
            serde_json::from_slice::<Value>(&bytes)?
                == expected_parts[part.part_number as usize - 1]
        );
    }
    for part in ["0", "3", "invalid"] {
        error(
            client
                .get(format!("{url}/parts/{part}"))
                .bearer_auth(TOKEN)
                .send()
                .await?,
            StatusCode::NOT_FOUND,
            "data_subject_operation_not_found",
        )
        .await?;
    }
    server.advance_time(Duration::from_secs(7 * 24 * 60 * 60))?;
    error(
        client
            .get(format!("{url}/parts/1"))
            .bearer_auth(TOKEN)
            .send()
            .await?,
        StatusCode::GONE,
        "data_subject_export_expired",
    )
    .await?;
    assert_eq!(poll(&client, &url).await?, done);
    server.advance_time(Duration::from_secs(400 * 24 * 60 * 60))?;
    error(
        client.get(&url).bearer_auth(TOKEN).send().await?,
        StatusCode::NOT_FOUND,
        "data_subject_operation_not_found",
    )
    .await?;
    let replacement = start(&client, &server, EXPORT, SUBJECT).await?;
    assert_ne!(replacement.operation_id, done.operation_id);
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
async fn receipt_faults_fail_at_the_intended_verification_step() -> Result {
    for fault in [
        Fault::BadSignature,
        Fault::WrongKeyId,
        Fault::TamperedPayload,
        Fault::WrongSubjectReference,
    ] {
        let server = target().await?;
        let client = client();
        // An all-x maximum-length reference catches an accidental unchanged replacement.
        let reference = "x".repeat(128);
        server.set_script(
            &reference,
            Script::new(Outcome::Completed {
                categories: categories(),
            })
            .with_fault(fault.clone()),
        )?;
        let pending = start(&client, &server, ERASE, &reference).await?;
        let done = poll(&client, &operation_url(&server, ERASE, &pending)).await?;
        let envelope = done.receipt.as_ref().expect("receipt");
        let verifier = ReceiptVerifier::from_discovery(&server.discovery())?;
        match fault {
            Fault::BadSignature | Fault::WrongKeyId => {
                assert_eq!(verifier.verify(envelope), Err(ReceiptError::UnknownKey));
                let mut corrected = envelope.clone();
                corrected.signatures[0].keyid = server.receipt_key().key_id;
                if matches!(fault, Fault::BadSignature) {
                    assert_eq!(verifier.verify(&corrected), Err(ReceiptError::Signature));
                } else {
                    assert!(verifier.verify(&corrected).is_ok());
                }
            }
            Fault::TamperedPayload => {
                serde_json::from_slice::<ReceiptPayload>(&STANDARD.decode(&envelope.payload)?)?;
                assert_eq!(verifier.verify(envelope), Err(ReceiptError::Signature));
            }
            Fault::WrongSubjectReference => {
                let payload = verifier.verify(envelope)?;
                assert!(payload.subject_reference != reference);
                assert!(done.subject_reference == reference);
            }
            _ => unreachable!("only receipt faults"),
        }
        let replay = post(&client, &server, ERASE, &reference)
            .await?
            .json::<OperationResource>()
            .await?;
        assert_eq!(replay, done);
    }
    Ok(())
}

#[tokio::test]
async fn part_digest_fault_preserves_valid_receipt_size_and_json() -> Result {
    let server = target().await?;
    let client = client();
    server.set_script(
        SUBJECT,
        Script::new(Outcome::Export {
            parts: vec![json!({"record": 1}), json!(0), json!(1)],
        })
        .with_fault(Fault::PartDigestMismatch),
    )?;
    let pending = start(&client, &server, EXPORT, SUBJECT).await?;
    let url = operation_url(&server, EXPORT, &pending);
    let done = poll(&client, &url).await?;
    let payload = verify(&server, &done)?;
    for part in payload.parts {
        let bytes = client
            .get(format!("{url}/parts/{}", part.part_number))
            .bearer_auth(TOKEN)
            .send()
            .await?
            .bytes()
            .await?;
        assert_eq!(bytes.len() as u64, part.bytes);
        assert_ne!(sha256(&bytes), part.sha256);
        serde_json::from_slice::<Value>(&bytes)?;
    }
    Ok(())
}

#[tokio::test]
async fn transient_faults_retry_post_poll_and_download_without_losing_state() -> Result {
    let server = target().await?;
    let client = client();
    server.set_script(
        SUBJECT,
        Script::new(Outcome::Export {
            parts: vec![json!([1])],
        })
        .with_fault(Fault::ServerError {
            status: 503,
            times: 2,
        })
        .with_fault(Fault::RateLimited {
            retry_after: Duration::from_millis(1500),
            times: 2,
        }),
    )?;
    for _ in 0..2 {
        error(
            post(&client, &server, EXPORT, SUBJECT).await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
        )
        .await?;
    }
    for _ in 0..2 {
        let response = post(&client, &server, EXPORT, SUBJECT).await?;
        assert_eq!(response.headers()["retry-after"], "2");
        error(response, StatusCode::TOO_MANY_REQUESTS, "rate_limited").await?;
    }
    let pending = start(&client, &server, EXPORT, SUBJECT).await?;
    let url = operation_url(&server, EXPORT, &pending);
    server.push_fault(
        SUBJECT,
        Fault::ServerError {
            status: 502,
            times: 1,
        },
    )?;
    error(
        client.get(&url).bearer_auth(TOKEN).send().await?,
        StatusCode::BAD_GATEWAY,
        "server_error",
    )
    .await?;
    let done = poll(&client, &url).await?;
    verify(&server, &done)?;
    server.push_fault(
        SUBJECT,
        Fault::RateLimited {
            retry_after: Duration::from_secs(3),
            times: 1,
        },
    )?;
    let part_url = format!("{url}/parts/1");
    let response = client.get(&part_url).bearer_auth(TOKEN).send().await?;
    assert_eq!(response.headers()["retry-after"], "3");
    error(response, StatusCode::TOO_MANY_REQUESTS, "rate_limited").await?;
    assert_eq!(
        client
            .get(&part_url)
            .bearer_auth(TOKEN)
            .send()
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post(&client, &server, EXPORT, SUBJECT)
            .await?
            .json::<OperationResource>()
            .await?,
        done
    );
    assert_eq!(
        server.push_fault("unconfigured-reference", Fault::BadSignature),
        Err(DataSubjectScriptError::MissingScript)
    );
    Ok(())
}

#[tokio::test]
async fn rotation_signs_later_completions_and_keeps_old_receipts() -> Result {
    let server = target().await?;
    let client = client();
    let old_discovery = server.discovery();
    let pending = start(&client, &server, ERASE, SUBJECT).await?;
    let done = poll(&client, &operation_url(&server, ERASE, &pending)).await?;
    let later = start(&client, &server, EXPORT, SUBJECT).await?;
    let key = server.rotate_key()?;
    assert_ne!(key.key_id, old_discovery.receipt_keys[0].key_id);
    assert_eq!(
        server.discovery().receipt_keys[1].status,
        ReceiptKeyStatus::Previous
    );
    let later = poll(&client, &operation_url(&server, EXPORT, &later)).await?;
    assert_eq!(
        later.receipt.as_ref().expect("receipt").signatures[0].keyid,
        key.key_id
    );
    assert_eq!(
        ReceiptVerifier::from_discovery(&old_discovery)?
            .verify(later.receipt.as_ref().expect("receipt")),
        Err(ReceiptError::UnknownKey)
    );
    verify(&server, &done)?;
    verify(&server, &later)?;
    server.rotate_key()?;
    assert_eq!(server.discovery().receipt_keys.len(), 3);
    verify(&server, &done)?;
    verify(&server, &later)?;
    assert_eq!(
        post(&client, &server, ERASE, SUBJECT)
            .await?
            .json::<OperationResource>()
            .await?,
        done
    );
    Ok(())
}

#[tokio::test]
async fn delay_allows_lost_response_reconciliation_and_request_observations() -> Result {
    let server = Server::start().await?;
    let token_id = server.register_token(TOKEN, "privacy-client", ["data-subject.erase"]);
    server.set_script(
        SUBJECT,
        Script::new(Outcome::NotFound).with_fault(Fault::Delay(Duration::from_millis(120))),
    )?;
    let client = client();
    let result = client
        .post(format!("{}{ERASE}", server.base_url()))
        .bearer_auth(TOKEN)
        .header("idempotency-key", KEY)
        .json(&body(SUBJECT))
        .timeout(Duration::from_millis(40))
        .send()
        .await;
    assert!(result.is_err_and(|error| error.is_timeout()));
    let before = tokio::time::Instant::now();
    let replay = start(&client, &server, ERASE, SUBJECT).await?;
    assert!(before.elapsed() >= Duration::from_millis(120));
    let url = operation_url(&server, ERASE, &replay);
    let done = poll(&client, &url).await?;
    assert_eq!(done.status, OperationStatus::NotFound);
    let requests = server.received_requests();
    assert_eq!(requests.len(), 3);
    for request in &requests[..2] {
        assert_eq!(request.method(), Method::POST);
        assert_eq!(request.path(), ERASE);
        assert!(request.idempotency_key() == Some(KEY));
        assert_eq!(request.token(), Some(token_id));
        assert!(request.body() == Some(&body(SUBJECT)));
    }
    assert_eq!(requests[2].method(), Method::GET);
    assert!(requests[2].body().is_none());
    assert!(requests[2].idempotency_key().is_none());
    error(
        client.get(&url).bearer_auth("unknown").send().await?,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await?;
    assert!(
        server
            .received_requests()
            .last()
            .expect("request")
            .token()
            .is_none()
    );
    verify(&server, &done)?;
    Ok(())
}

#[tokio::test]
async fn drop_closes_listener_keep_alive_and_delayed_requests() -> Result {
    let server = target().await?;
    let client = client();
    server.set_script(
        SUBJECT,
        Script::new(Outcome::NotFound).with_fault(Fault::Delay(Duration::from_secs(30))),
    )?;
    let url = format!("{}{ERASE}", server.base_url());
    let port = reqwest::Url::parse(server.base_url())?
        .port()
        .expect("loopback port");
    let mut waiting = Box::pin(
        client
            .post(url)
            .bearer_auth(TOKEN)
            .header("idempotency-key", KEY)
            .json(&body(SUBJECT))
            .send(),
    );
    tokio::select! {
        response = &mut waiting => panic!("delayed request returned early: {}", response.is_ok()),
        _ = async {
            while server.received_requests().is_empty() { tokio::task::yield_now().await; }
        } => {}
    }
    let discovery_url = format!("{}/.well-known/baukit/data-subject", server.base_url());
    assert_eq!(
        client.get(&discovery_url).send().await?.status(),
        StatusCode::OK
    );
    drop(server);
    tokio::time::timeout(Duration::from_secs(1), async {
        let _ = waiting.await;
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(client.get(discovery_url).send().await.is_err());
    Ok(())
}
