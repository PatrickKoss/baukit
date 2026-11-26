use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Extension, Router,
    extract::{Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use baukit_auth::{OidcConfig, OidcVerifier};
use baukit_test::{
    AuthNegativeCase, CrossTenantCase, MockOidcServer, OperationFixtures, SecurityTarget,
    assert_cross_tenant_denied, check_auth_negative_suite, check_cross_tenant_denied,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;

const AUDIENCE: &str = "orders-api";
const ALICE: &str = "alice";
const BOB: &str = "bob";
const ALICE_ITEM: &str = "item-of-alice";
const ALICE_NOTE: &str = "note-of-alice";
const ALICE_SECRET: &str = "alice-private-text";

#[derive(Clone, Debug)]
struct Subject(String);

async fn authenticate(
    State(verifier): State<Arc<OidcVerifier>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(token) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return challenge();
    };
    match verifier.verify(token).await {
        Ok(principal) => {
            let subject = Subject(principal.subject().to_owned());
            request.extensions_mut().insert(subject);
            next.run(request).await
        }
        Err(_) => challenge(),
    }
}

fn challenge() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}

async fn owned_item(Extension(subject): Extension<Subject>, Path(id): Path<String>) -> Response {
    if id == ALICE_ITEM && subject.0 == ALICE {
        return json_body(json!({"id": id, "text": ALICE_SECRET}));
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn note_without_owner_check(Path(id): Path<String>) -> Response {
    if id == ALICE_NOTE {
        return json_body(json!({"id": id, "text": ALICE_SECRET}));
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn shared_export(Path(id): Path<String>) -> Response {
    json_body(json!({"id": id, "public": true}))
}

fn json_body(body: Value) -> Response {
    axum::Json(body).into_response()
}

async fn app(issuer: &MockOidcServer) -> Result<Router, Box<dyn Error>> {
    let verifier =
        Arc::new(OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?);
    let secured = Router::new()
        .route("/items/{itemId}", get(owned_item).delete(owned_item))
        .route("/notes/{noteId}", get(note_without_owner_check))
        .route("/exports/{exportId}", get(shared_export))
        .layer(middleware::from_fn_with_state(verifier, authenticate));
    Ok(Router::new()
        .route("/unguarded/{itemId}", get(owned_item_without_auth))
        .route("/health", get(|| async { "ok" }))
        .merge(secured))
}

async fn owned_item_without_auth(Path(id): Path<String>) -> Response {
    json_body(json!({"id": id}))
}

fn secured() -> Value {
    json!([{"bearer": []}])
}

fn document(paths: &[&str]) -> Value {
    let mut documented = serde_json::Map::new();
    for path in paths {
        let item = match *path {
            "/items/{itemId}" => json!({"get": {}, "delete": {}}),
            "/health" => json!({"get": {"security": []}}),
            _ => json!({"get": {}}),
        };
        documented.insert((*path).to_owned(), item);
    }
    json!({"openapi": "3.1.0", "security": secured(), "paths": documented})
}

fn bob_token(issuer: &MockOidcServer) -> Result<String, Box<dyn Error>> {
    Ok(issuer.mint(&issuer.claims(BOB, AUDIENCE, Duration::from_secs(300))?)?)
}

fn alice_fixtures() -> OperationFixtures {
    OperationFixtures::new()
        .path_value("itemId", ALICE_ITEM)
        .path_value("noteId", ALICE_NOTE)
        .path_value("exportId", "export-1")
}

async fn serve(router: Router) -> Result<SecurityTarget, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(SecurityTarget::base_url(format!("http://{address}")))
}

#[tokio::test]
async fn owner_scoped_routes_pass_in_process_and_over_http() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let router = app(&issuer).await?;
    let alice = issuer.mint(&issuer.claims(ALICE, AUDIENCE, Duration::from_secs(300))?)?;
    let alice_view = check_cross_tenant_denied(
        &document(&["/items/{itemId}"]),
        &SecurityTarget::router(router.clone()),
        &alice_fixtures(),
        &CrossTenantCase::new(alice),
    )
    .await
    .err()
    .ok_or("the owner must reach the item, proving the fixture id is real")?;
    assert_eq!(alice_view.violations().len(), 2, "{alice_view}");

    let doc = document(&["/items/{itemId}", "/health"]);
    let case = CrossTenantCase::new(bob_token(&issuer)?).leak_marker(ALICE_SECRET);
    for target in [SecurityTarget::router(router.clone()), serve(router).await?] {
        let report = assert_cross_tenant_denied(&doc, &target, &alice_fixtures(), &case).await;
        assert_eq!(
            report.probed,
            [
                "GET /health",
                "GET /items/{itemId}",
                "DELETE /items/{itemId}"
            ]
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_route_without_owner_check_is_reported() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let target = SecurityTarget::router(app(&issuer).await?);
    let doc = document(&["/items/{itemId}", "/notes/{noteId}"]);
    let error = check_cross_tenant_denied(
        &doc,
        &target,
        &alice_fixtures(),
        &CrossTenantCase::new(bob_token(&issuer)?),
    )
    .await
    .err()
    .ok_or("the note route skips the owner check")?;
    assert_eq!(
        error.violations(),
        ["GET /notes/{noteId}: status 200, expected one of [403, 404]"]
    );

    let leaked = check_cross_tenant_denied(
        &doc,
        &target,
        &alice_fixtures(),
        &CrossTenantCase::new(bob_token(&issuer)?)
            .denied_statuses([200, 403, 404])
            .leak_marker(ALICE_SECRET),
    )
    .await
    .err()
    .ok_or("the body carries alice's text")?;
    assert_eq!(
        leaked.violations(),
        [format!(
            "GET /notes/{{noteId}}: status 200 leaked `{ALICE_SECRET}` in the body"
        )]
    );
    Ok(())
}

#[tokio::test]
async fn excluded_routes_are_skipped_and_reported_with_their_reason() -> Result<(), Box<dyn Error>>
{
    let issuer = MockOidcServer::start().await?;
    let target = SecurityTarget::router(app(&issuer).await?);
    let reason = "exports are shared links readable by every signed-in user";
    let report = assert_cross_tenant_denied(
        &document(&["/items/{itemId}", "/exports/{exportId}"]),
        &target,
        &alice_fixtures(),
        &CrossTenantCase::new(bob_token(&issuer)?).exclude("GET", "/exports/{exportId}", reason),
    )
    .await;
    assert_eq!(report.excluded.len(), 1);
    assert_eq!(
        report
            .excluded
            .first()
            .map(|exclusion| exclusion.reason.as_str()),
        Some(reason)
    );
    assert!(
        report
            .to_string()
            .contains(&format!("excluded GET /exports/{{exportId}}: {reason}"))
    );
    assert!(!report.probed.iter().any(|route| route.contains("exports")));

    let error = check_cross_tenant_denied(
        &document(&["/items/{itemId}"]),
        &target,
        &alice_fixtures(),
        &CrossTenantCase::new(bob_token(&issuer)?)
            .exclude("get", "/gone/{id}", "removed route")
            .exclude("delete", "/items/{itemId}", " "),
    )
    .await
    .err()
    .ok_or("stale and reasonless exclusions fail")?;
    assert_eq!(
        error.violations(),
        [
            "exclusion GET /gone/{id} names no documented operation",
            "exclusion DELETE /items/{itemId} has no reason",
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_route_without_fixture_id_fails_instead_of_being_skipped() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let error = check_cross_tenant_denied(
        &document(&["/items/{itemId}", "/exports/{exportId}"]),
        &SecurityTarget::router(app(&issuer).await?),
        &OperationFixtures::new().path_value("itemId", ALICE_ITEM),
        &CrossTenantCase::new(bob_token(&issuer)?),
    )
    .await
    .err()
    .ok_or("exportId has no fixture")?;
    assert_eq!(
        error.violations(),
        ["GET /exports/{exportId}: no fixture value for path parameter `exportId`"]
    );
    Ok(())
}

#[tokio::test]
async fn auth_negative_suite_passes_on_verified_routes() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let router = app(&issuer).await?;
    let doc = document(&["/items/{itemId}", "/notes/{noteId}", "/health"]);
    let case = AuthNegativeCase::new(&issuer, AUDIENCE).public("get", "/health", "liveness probe");
    for target in [SecurityTarget::router(router.clone()), serve(router).await?] {
        let report = check_auth_negative_suite(&doc, &target, &OperationFixtures::new(), &case)
            .await
            .map_err(|error| error.to_string())?;
        assert_eq!(
            report.probed,
            [
                "GET /items/{itemId}",
                "DELETE /items/{itemId}",
                "GET /notes/{noteId}"
            ]
        );
    }
    Ok(())
}

#[tokio::test]
async fn auth_negative_suite_reports_every_credential_an_unguarded_route_accepts()
-> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let target = SecurityTarget::router(app(&issuer).await?);
    let doc = document(&["/items/{itemId}", "/unguarded/{itemId}"]);
    let error = check_auth_negative_suite(
        &doc,
        &target,
        &alice_fixtures(),
        &AuthNegativeCase::new(&issuer, AUDIENCE),
    )
    .await
    .err()
    .ok_or("the unguarded route never checks the token")?;
    assert_eq!(
        error.violations(),
        [
            "GET /unguarded/{itemId}: missing token got status 200, expected 401",
            "GET /unguarded/{itemId}: expired token got status 200, expected 401",
            "GET /unguarded/{itemId}: other-audience token got status 200, expected 401",
            "GET /unguarded/{itemId}: unknown-key token got status 200, expected 401",
            "GET /unguarded/{itemId}: alg none token got status 200, expected 401",
            "GET /unguarded/{itemId}: wrong-issuer token got status 200, expected 401",
            "GET /unguarded/{itemId}: not-yet-valid token got status 200, expected 401",
            "GET /unguarded/{itemId}: hs256 public-key token got status 200, expected 401",
            "GET /unguarded/{itemId}: random-kid token got status 200, expected 401",
            "GET /unguarded/{itemId}: duplicate authorization got status 200, expected 401",
            "GET /unguarded/{itemId}: malformed authorization got status 200, expected 401",
        ]
    );

    let excluded = check_auth_negative_suite(
        &doc,
        &target,
        &alice_fixtures(),
        &AuthNegativeCase::new(&issuer, AUDIENCE).exclude(
            "get",
            "/unguarded/{itemId}",
            "fixture route without auth layer",
        ),
    )
    .await
    .map_err(|error| error.to_string())?;
    assert_eq!(
        excluded.probed,
        ["GET /items/{itemId}", "DELETE /items/{itemId}"]
    );
    Ok(())
}

#[tokio::test]
async fn unsecured_routes_must_be_declared_public_and_public_routes_must_be_unsecured()
-> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let target = SecurityTarget::router(app(&issuer).await?);
    let doc = document(&["/items/{itemId}", "/health"]);
    let undeclared = check_auth_negative_suite(
        &doc,
        &target,
        &alice_fixtures(),
        &AuthNegativeCase::new(&issuer, AUDIENCE),
    )
    .await
    .err()
    .ok_or("an undeclared unsecured route fails")?;
    assert_eq!(
        undeclared.violations(),
        ["GET /health: documented without security; declare it with .public(method, path, reason)"]
    );
    let wrong = check_auth_negative_suite(
        &doc,
        &target,
        &alice_fixtures(),
        &AuthNegativeCase::new(&issuer, AUDIENCE)
            .public("get", "/health", "liveness probe")
            .public("get", "/items/{itemId}", "claimed public"),
    )
    .await
    .err()
    .ok_or("a secured route declared public fails")?;
    assert_eq!(
        wrong.violations(),
        ["GET /items/{itemId}: declared public but documented as secured"]
    );
    Ok(())
}

#[tokio::test]
async fn a_route_rejecting_every_caller_fails_the_positive_control() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let router = Router::new().route("/locked/{itemId}", get(|| async { challenge() }));
    let error = check_auth_negative_suite(
        &document(&["/locked/{itemId}"]),
        &SecurityTarget::router(router),
        &alice_fixtures(),
        &AuthNegativeCase::new(&issuer, AUDIENCE),
    )
    .await
    .err()
    .ok_or("a route that rejects valid tokens is not proof of auth")?;
    assert_eq!(
        error.violations(),
        [
            "GET /locked/{itemId}: valid token got 401, so the route rejects every caller (positive control)"
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_401_without_www_authenticate_is_reported() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let verifier =
        Arc::new(OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?);
    let router = Router::new()
        .route("/items/{itemId}", get(owned_item).delete(owned_item))
        .layer(middleware::from_fn_with_state(verifier, bare_401));
    let error = check_auth_negative_suite(
        &document(&["/items/{itemId}"]),
        &SecurityTarget::router(router),
        &alice_fixtures(),
        &AuthNegativeCase::new(&issuer, AUDIENCE),
    )
    .await
    .err()
    .ok_or("missing challenge header")?;
    assert_eq!(
        error.violations(),
        [
            "GET /items/{itemId}: missing token got 401 without WWW-Authenticate",
            "DELETE /items/{itemId}: missing token got 401 without WWW-Authenticate",
        ]
    );
    Ok(())
}

async fn bare_401(
    State(verifier): State<Arc<OidcVerifier>>,
    request: Request,
    next: Next,
) -> Response {
    if request.headers().contains_key(header::AUTHORIZATION) {
        return authenticate(State(verifier), request, next).await;
    }
    StatusCode::UNAUTHORIZED.into_response()
}

type Shelf = Arc<Mutex<Option<String>>>;

async fn shelf_read(
    State(shelf): State<Shelf>,
    Extension(subject): Extension<Subject>,
) -> Response {
    let stored = shelf.lock().ok().and_then(|text| text.clone());
    match stored {
        Some(text) if subject.0 == ALICE => json_body(json!({"text": text})),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn shelf_delete_without_owner_check(State(shelf): State<Shelf>) -> Response {
    if let Ok(mut text) = shelf.lock() {
        *text = None;
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn shelf_listing(State(shelf): State<Shelf>) -> Response {
    let stored = shelf.lock().ok().and_then(|text| text.clone());
    json_body(json!({"items": stored}))
}

#[tokio::test]
async fn the_owner_rereads_after_each_denied_request_and_listings_are_checked_for_leaks()
-> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let verifier =
        Arc::new(OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?);
    let shelf: Shelf = Arc::new(Mutex::new(Some(ALICE_SECRET.to_owned())));
    let router = Router::new()
        .route(
            "/shelves/{itemId}",
            get(shelf_read).delete(shelf_delete_without_owner_check),
        )
        .route("/shelves", get(shelf_listing))
        .with_state(shelf)
        .layer(middleware::from_fn_with_state(verifier, authenticate));
    let alice = issuer.mint(&issuer.claims(ALICE, AUDIENCE, Duration::from_secs(300))?)?;
    let doc = json!({"openapi": "3.1.0", "security": secured(), "paths": {
        "/shelves": {"get": {}},
        "/shelves/{itemId}": {"get": {}, "delete": {}},
    }});
    let error = check_cross_tenant_denied(
        &doc,
        &SecurityTarget::router(router),
        &alice_fixtures(),
        &CrossTenantCase::new(bob_token(&issuer)?)
            .owner(alice)
            .leak_marker(ALICE_SECRET),
    )
    .await
    .err()
    .ok_or("the delete skips the owner check and the listing leaks")?;
    assert_eq!(
        error.violations(),
        [
            format!("GET /shelves: status 200 leaked `{ALICE_SECRET}` in the body"),
            "DELETE /shelves/{itemId}: owner's GET /shelves/{itemId} changed after the denied request".to_owned(),
        ]
    );
    Ok(())
}
