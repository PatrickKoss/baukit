#![cfg(feature = "data-subject")]

use std::{
    convert::Infallible,
    io,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header},
    response::Response,
};
use baukit_data_subject::{
    CategoryAction, CategoryEntry, Identifier, IdentifierType, OperationKind, Restriction,
};
use baukit_test::{
    DataSubjectConformanceAdapter, DataSubjectConformanceError as Failure, DataSubjectEndpoint,
    DataSubjectFault, DataSubjectOutcome, DataSubjectScript, DataSubjectTargetServer,
    DataSubjectTestClient, DataSubjectTestSubject, check_data_subject_conformance,
};
use serde_json::{Value, json};
use uuid::Uuid;

struct Adapter {
    target: DataSubjectTargetServer,
    fault: Option<DataSubjectFault>,
    restrictions: bool,
    seeds: usize,
    drains: usize,
}

impl Adapter {
    async fn new(fault: Option<DataSubjectFault>) -> io::Result<Self> {
        Ok(Self {
            target: DataSubjectTargetServer::start().await?,
            fault,
            restrictions: true,
            seeds: 0,
            drains: 0,
        })
    }
}

#[async_trait::async_trait]
impl DataSubjectConformanceAdapter for Adapter {
    type Error = Infallible;

    async fn mint_token(
        &mut self,
        client: DataSubjectTestClient,
        scopes: &[&str],
    ) -> Result<String, Self::Error> {
        let token = Uuid::new_v4().to_string();
        let client = match client {
            DataSubjectTestClient::Allowed => "privacy",
            DataSubjectTestClient::OtherAllowed => "other-privacy",
            DataSubjectTestClient::Disallowed | DataSubjectTestClient::ApiToken => "",
        };
        self.target
            .register_token(&token, client, scopes.iter().copied());
        Ok(token)
    }

    async fn seed_subject(
        &mut self,
        kind: OperationKind,
        case: DataSubjectTestSubject,
        reference: &str,
    ) -> Result<Option<Vec<Identifier>>, Self::Error> {
        self.seeds += 1;
        let categories = vec![CategoryEntry {
            category: "records".into(),
            action: if kind == OperationKind::Erasure {
                CategoryAction::Erased
            } else {
                CategoryAction::Exported
            },
            count: 1,
        }];
        let outcome = match case {
            DataSubjectTestSubject::Matched if kind == OperationKind::Export => {
                DataSubjectOutcome::Export {
                    parts: vec![json!({"records":[1,2]}), json!({"records":[3]})],
                }
            }
            DataSubjectTestSubject::Matched => DataSubjectOutcome::Completed { categories },
            DataSubjectTestSubject::Unmatched => DataSubjectOutcome::NotFound,
            DataSubjectTestSubject::Restricted if !self.restrictions => return Ok(None),
            DataSubjectTestSubject::Restricted => DataSubjectOutcome::Restricted {
                categories: vec![CategoryEntry {
                    category: "invoices".into(),
                    action: CategoryAction::Retained,
                    count: 1,
                }],
                restrictions: vec![Restriction {
                    category: "invoices".into(),
                    reason: "Tax retention".into(),
                    legal_basis: "Tax code".into(),
                    retain_until: None,
                }],
            },
        };
        let mut script = DataSubjectScript::new(outcome);
        if case == DataSubjectTestSubject::Matched
            && let Some(fault) = &self.fault
        {
            script = script.with_fault(fault.clone());
        }
        assert!(self.target.set_script(reference, script).is_ok());
        Ok(Some(vec![Identifier {
            identifier_type: IdentifierType::Email,
            value: format!("lookup-{}@example.invalid", Uuid::new_v4()),
        }]))
    }

    async fn run_pending_work(&mut self) -> Result<(), Self::Error> {
        self.drains += 1;
        // This target completes on the next status GET without background work.
        Ok(())
    }
}

#[tokio::test]
async fn controlled_target_conforms_over_http() -> Result<(), Box<dyn std::error::Error>> {
    let mut adapter = Adapter::new(None).await?;
    let base = adapter.target.base_url().to_owned();
    check_data_subject_conformance(DataSubjectEndpoint::BaseUrl(&base), &mut adapter).await?;
    assert_eq!(adapter.seeds, 5);
    assert_eq!(adapter.drains, 5);
    Ok(())
}

#[tokio::test]
async fn unsupported_restrictions_are_skipped() -> Result<(), Box<dyn std::error::Error>> {
    let mut adapter = Adapter::new(None).await?;
    adapter.restrictions = false;
    let base = adapter.target.base_url().to_owned();
    check_data_subject_conformance(DataSubjectEndpoint::BaseUrl(&base), &mut adapter).await?;
    assert_eq!(adapter.drains, 4);
    Ok(())
}

#[tokio::test]
async fn rejects_receipt_and_part_faults_with_matching_requirements()
-> Result<(), Box<dyn std::error::Error>> {
    for (fault, expected) in [
        (DataSubjectFault::BadSignature, Failure::Receipt),
        (DataSubjectFault::WrongKeyId, Failure::Receipt),
        (DataSubjectFault::TamperedPayload, Failure::Receipt),
        (
            DataSubjectFault::WrongSubjectReference,
            Failure::ReceiptFields,
        ),
        (DataSubjectFault::PartDigestMismatch, Failure::ExportPart),
    ] {
        let mut adapter = Adapter::new(Some(fault)).await?;
        let base = adapter.target.base_url().to_owned();
        let result =
            check_data_subject_conformance(DataSubjectEndpoint::BaseUrl(&base), &mut adapter).await;
        assert_eq!(result, Err(expected));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Mutation {
    Cache,
    ImplicitPublicCache,
    TerminalReplay,
    Version,
    Keys,
    Scopes,
    Route,
    BadBodyStatus,
    Echo,
    EscapedEcho,
    ReceiptEcho,
    NoAuth,
    Key,
    Limit,
    Conflict,
    OtherClient,
    ReceiptFields,
}

#[derive(Clone)]
struct Proxy {
    base: String,
    client: reqwest::Client,
    mutation: Mutation,
    lookup: Arc<Mutex<Option<String>>>,
    seen: Arc<Mutex<std::collections::HashSet<String>>>,
}

async fn proxy(State(state): State<Proxy>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    let body = to_bytes(body, 1_048_576)
        .await
        .expect("bounded fixture request");
    let submitted = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    if let Some(value) = submitted["identifiers"][0]["value"].as_str() {
        *state.lookup.lock().expect("lookup lock") = Some(value.to_owned());
    }
    let authorization = parts.headers.get(header::AUTHORIZATION).cloned();
    let response = state
        .client
        .request(parts.method.clone(), format!("{}{path}", state.base))
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .expect("fixture response");
    let mut status = response.status();
    let mut headers = response.headers().clone();
    headers.remove(header::CONTENT_LENGTH);
    let bytes = response.bytes().await.expect("fixture body");
    let mut json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if matches!(state.mutation, Mutation::TerminalReplay)
        && parts.method == "POST"
        && status == StatusCode::ACCEPTED
    {
        let id = json["operationId"]
            .as_str()
            .expect("operation ID")
            .to_owned();
        let seen = !state.seen.lock().expect("seen lock").insert(id.clone());
        if seen {
            // Complete the target between acceptance and the first replay response.
            let response = state
                .client
                .get(format!("{}{path}/{id}", state.base))
                .header(
                    header::AUTHORIZATION,
                    authorization.expect("authorized replay"),
                )
                .send()
                .await
                .expect("complete replay");
            status = StatusCode::OK;
            json = response.json().await.expect("terminal operation");
        }
    }
    if path == "/.well-known/baukit/data-subject" {
        match state.mutation {
            Mutation::Cache => {
                headers.insert(header::CACHE_CONTROL, "no-store".parse().expect("header"));
            }
            Mutation::ImplicitPublicCache => {
                headers.insert(
                    header::CACHE_CONTROL,
                    "max-age=300".parse().expect("header"),
                );
            }
            Mutation::Version => json["contractVersion"] = json!("2"),
            Mutation::Keys => json["receiptKeys"] = json!([]),
            Mutation::Scopes => json["scopes"]["erasure"] = json!("wrong.scope"),
            _ => {}
        }
    } else {
        let code = json["error"]["code"].as_str().unwrap_or("").to_owned();
        match state.mutation {
            Mutation::Route => status = StatusCode::NOT_FOUND,
            Mutation::NoAuth if status == StatusCode::UNAUTHORIZED => status = StatusCode::OK,
            Mutation::Key if code == "data_subject_idempotency_key_invalid" => {
                status = StatusCode::UNPROCESSABLE_ENTITY
            }
            Mutation::Limit
                if submitted["identifiers"]
                    .as_array()
                    .is_some_and(|v| v.len() > 16) =>
            {
                status = StatusCode::BAD_REQUEST
            }
            Mutation::BadBodyStatus if code == "data_subject_request_invalid" => {
                status = StatusCode::BAD_REQUEST
            }
            Mutation::Echo | Mutation::EscapedEcho
                if json["error"].is_object()
                    && submitted["identifiers"][0]["value"].is_string() =>
            {
                json["error"]["message"] = submitted["identifiers"][0]["value"].clone();
            }
            Mutation::ReceiptEcho if json["receipt"].is_object() => {
                use base64::{Engine as _, engine::general_purpose::STANDARD};
                json["receipt"]["payload"] = json!(STANDARD.encode(serde_json::to_vec(&json!({"lookup": state.lookup.lock().expect("lookup lock").as_deref().expect("submitted lookup")})).expect("JSON")));
            }
            Mutation::Conflict if code == "data_subject_idempotency_conflict" => {
                status = StatusCode::BAD_REQUEST
            }
            Mutation::OtherClient if code == "data_subject_operation_not_found" => {
                status = StatusCode::OK
            }
            Mutation::ReceiptFields if json["status"] == "completed" => {
                json["categories"] = json!([])
            }
            _ => {}
        }
    }
    let mut bytes = serde_json::to_vec(&json).expect("JSON");
    if matches!(state.mutation, Mutation::EscapedEcho) {
        let text = String::from_utf8(bytes).expect("JSON UTF-8");
        bytes = text.replace("lookup-", "\\u006cookup-").into_bytes();
    }
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

#[tokio::test]
async fn rejects_broken_http_requirements_without_disclosing_inputs()
-> Result<(), Box<dyn std::error::Error>> {
    for (mutation, expected) in [
        (Mutation::Cache, Failure::CacheControl),
        (Mutation::Version, Failure::ContractVersion),
        (Mutation::Keys, Failure::ReceiptKeys),
        (Mutation::Scopes, Failure::Scopes),
        (Mutation::Route, Failure::Routes),
        (Mutation::BadBodyStatus, Failure::InvalidBody),
        (Mutation::Echo, Failure::IdentifierLeak),
        (Mutation::EscapedEcho, Failure::IdentifierLeak),
        (Mutation::ReceiptEcho, Failure::IdentifierLeak),
        (Mutation::NoAuth, Failure::Unauthorized),
        (Mutation::Key, Failure::IdempotencyKey),
        (Mutation::Limit, Failure::Limits),
        (Mutation::Conflict, Failure::Conflict),
        (Mutation::OtherClient, Failure::Isolation),
        (Mutation::ReceiptFields, Failure::ReceiptFields),
    ] {
        let mut adapter = Adapter::new(None).await?;
        let router = Router::new().fallback(proxy).with_state(Proxy {
            base: adapter.target.base_url().to_owned(),
            client: reqwest::Client::builder().no_proxy().build()?,
            mutation,
            lookup: Arc::new(Mutex::new(None)),
            seen: Arc::new(Mutex::new(std::collections::HashSet::new())),
        });
        let error =
            check_data_subject_conformance(DataSubjectEndpoint::Router(&router), &mut adapter)
                .await
                .expect_err("broken contract");
        assert_eq!(error, expected);
        assert!(!format!("{error:?} {error}").contains("lookup-"));
    }
    Ok(())
}

#[tokio::test]
async fn accepts_terminal_replay_and_cacheable_discovery_without_public_directive()
-> Result<(), Box<dyn std::error::Error>> {
    for mutation in [Mutation::TerminalReplay, Mutation::ImplicitPublicCache] {
        let mut adapter = Adapter::new(None).await?;
        let router = Router::new().fallback(proxy).with_state(Proxy {
            base: adapter.target.base_url().to_owned(),
            client: reqwest::Client::builder().no_proxy().build()?,
            mutation,
            lookup: Arc::new(Mutex::new(None)),
            seen: Arc::new(Mutex::new(std::collections::HashSet::new())),
        });
        check_data_subject_conformance(DataSubjectEndpoint::Router(&router), &mut adapter).await?;
        assert_eq!(adapter.drains, 5);
    }
    Ok(())
}
