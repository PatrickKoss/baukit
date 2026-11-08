#![cfg(test)]
use baukit_erasure::{ErasureFuture, IdentityAccountDeleter, IdentityDeletionError};
use baukit_test::{
    FakeIdentityAccountDeleter, IdentityErasureAdapter, IdentityErasureRequest,
    IdentityErasureResponse, IdentityErasureRoutes, IdentityErasureSnapshot,
    check_identity_erasure_conformance,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const COMPLETED_AT: &str = "2026-11-02T08:00:00Z";

#[derive(Default)]
struct Product {
    subject: String,
    rows: u64,
    fenced: bool,
    operation: Option<Uuid>,
    completed: bool,
    keys: HashMap<String, String>,
}

struct FakeProduct {
    routes: IdentityErasureRoutes,
    deleter: Option<Arc<FakeIdentityAccountDeleter>>,
    state: Mutex<Product>,
    requests: Mutex<Vec<String>>,
}
impl FakeProduct {
    fn serving(routes: &IdentityErasureRoutes) -> Self {
        Self {
            routes: routes.clone(),
            deleter: None,
            state: Mutex::new(Product::default()),
            requests: Mutex::new(Vec::new()),
        }
    }
    fn state(&self) -> std::sync::MutexGuard<'_, Product> {
        self.state.lock().expect("state")
    }
    fn receipt(&self, product: &Product) -> IdentityErasureResponse {
        let operation = product.operation.expect("operation").to_string();
        if product.completed {
            response(
                200,
                json!({"status": "completed", "operationId": operation, "completedAt": COMPLETED_AT}),
            )
        } else {
            response(202, json!({"status": "pending", "operationId": operation}))
        }
    }
    async fn erase(&self, subject: &str, key: &str) -> IdentityErasureResponse {
        let owner = self.state().keys.get(key).cloned();
        match owner {
            Some(owner) if owner != subject => response(
                409,
                json!({"error": {"code": "erasure_idempotency_conflict"}}),
            ),
            Some(_) => self.receipt(&self.state()),
            None => {
                let deleter = self.deleter.clone().expect("seeded");
                let deleted = deleter.delete_account(subject).await.is_ok();
                let mut product = self.state();
                product.keys.insert(key.to_owned(), subject.to_owned());
                product.rows = 0;
                product.fenced = true;
                product.operation = Some(Uuid::now_v7());
                product.completed = deleted;
                self.receipt(&product)
            }
        }
    }
    fn status(&self, subject: &str, operation: &str) -> IdentityErasureResponse {
        let product = self.state();
        if subject != product.subject
            || product.operation.map(|id| id.to_string()).as_deref() != Some(operation)
        {
            return response(404, json!({"error": {"code": "not_found"}}));
        }
        let mut receipt = self.receipt(&product);
        receipt.status = 200;
        receipt
    }
    fn resolve(&self, subject: &str) -> IdentityErasureResponse {
        let product = self.state();
        if subject == product.subject && product.fenced {
            return response(401, json!({"error": {"code": "profile_erased"}}));
        }
        response(200, json!({"subject": subject}))
    }
}
fn response(status: u16, body: Value) -> IdentityErasureResponse {
    IdentityErasureResponse { status, body }
}
fn ready<'a, T: Send + 'a>(value: T) -> ErasureFuture<'a, Result<T, std::io::Error>> {
    Box::pin(async move { Ok(value) })
}

impl IdentityErasureAdapter for FakeProduct {
    type Error = std::io::Error;
    fn seed<'a>(
        &'a mut self,
        subject: &'a str,
        fake: Arc<FakeIdentityAccountDeleter>,
    ) -> ErasureFuture<'a, Result<(), Self::Error>> {
        self.deleter = Some(fake);
        *self.state() = Product {
            subject: subject.to_owned(),
            rows: 3,
            ..Product::default()
        };
        ready(())
    }
    fn request<'a>(
        &'a self,
        subject: &'a str,
        request: IdentityErasureRequest<'a>,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>> {
        Box::pin(async move {
            self.requests
                .lock()
                .expect("requests")
                .push(format!("{} {}", request.method, request.path));
            let routes = &self.routes;
            let operation = request
                .path
                .strip_prefix(routes.status_path.trim_end_matches("{operation}"));
            let erases = request.method == routes.erase_method && request.path == routes.erase_path;
            Ok(match (request.method, request.idempotency_key) {
                (_, Some(key)) if erases => self.erase(subject, key).await,
                ("GET", None) if request.path == routes.resolve_path => self.resolve(subject),
                ("GET", None) if operation.is_some_and(|id| Uuid::parse_str(id).is_ok()) => {
                    self.status(subject, operation.unwrap_or_default())
                }
                _ => response(404, json!({"error": {"code": "route_not_found"}})),
            })
        })
    }
    fn run_worker(&self) -> ErasureFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let subject = self.state().subject.clone();
            let deleter = self.deleter.clone().expect("seeded");
            if deleter.delete_account(&subject).await == Err(IdentityDeletionError::Retryable) {
                return Err(std::io::Error::other("identity deletion still failing"));
            }
            self.state().completed = true;
            Ok(())
        })
    }
    fn snapshot<'a>(
        &'a self,
        _subject: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureSnapshot, Self::Error>> {
        let product = self.state();
        let jobs = u64::from(product.fenced && !product.completed);
        ready(IdentityErasureSnapshot {
            product_rows: product.rows,
            identity_jobs: jobs,
            raw_subject_rows: product.rows + jobs,
            fenced: product.fenced,
        })
    }
}

#[test]
fn default_routes_are_the_template_routes() {
    assert_eq!(
        IdentityErasureRoutes::default(),
        IdentityErasureRoutes {
            erase_method: "DELETE".into(),
            erase_path: "/me".into(),
            status_path: "/me/erasures/{operation}".into(),
            resolve_path: "/me".into(),
        }
    );
}

#[tokio::test]
async fn conformance_calls_the_routes_the_caller_passes() {
    let routes = IdentityErasureRoutes {
        erase_method: "POST".into(),
        erase_path: "/v1/me/erasure".into(),
        status_path: "/v1/me/erasure/{operation}".into(),
        resolve_path: "/v1/me".into(),
    };
    let mut product = FakeProduct::serving(&routes);
    check_identity_erasure_conformance(&mut product, &routes, "subject-a", "subject-b")
        .await
        .expect("conformance");
    let requests = product.requests.lock().expect("requests").clone();
    assert!(
        requests
            .iter()
            .all(|line| line.starts_with("POST /v1/me/erasure") || line.starts_with("GET /v1/me"))
    );
    assert!(requests.contains(&"POST /v1/me/erasure".to_owned()));
    assert!(requests.contains(&"GET /v1/me".to_owned()));
    assert!(
        requests
            .iter()
            .any(|line| line.starts_with("GET /v1/me/erasure/"))
    );
}

#[tokio::test]
async fn conformance_fails_when_the_product_serves_other_routes() {
    let routes = IdentityErasureRoutes {
        erase_method: "POST".into(),
        erase_path: "/v1/me/erasure".into(),
        status_path: "/v1/me/erasure/{operation}".into(),
        resolve_path: "/v1/me".into(),
    };
    let mut product = FakeProduct::serving(&IdentityErasureRoutes::default());
    let error = check_identity_erasure_conformance(&mut product, &routes, "subject-a", "subject-b")
        .await
        .expect_err("template-only product must not conform at custom routes");
    assert!(error.to_string().contains("durable pending acceptance"));
}

#[tokio::test]
async fn conformance_rejects_a_status_path_without_the_placeholder() {
    let routes = IdentityErasureRoutes {
        status_path: "/me/erasures".into(),
        ..IdentityErasureRoutes::default()
    };
    let mut product = FakeProduct::serving(&IdentityErasureRoutes::default());
    let error = check_identity_erasure_conformance(&mut product, &routes, "subject-a", "subject-b")
        .await
        .expect_err("status path without placeholder");
    assert!(error.to_string().contains("{operation} placeholder"));
    assert!(product.requests.lock().expect("requests").is_empty());
}
