//! A scripted [`IntegrationConnector`] for testing job handlers without a
//! network.

use std::{
    collections::{BTreeMap, VecDeque},
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use baukit_integrations::{
    ClaimedConnectorJob, ConnectorError, ConnectorFuture, ConnectorPage, IntegrationConnector,
    RetryClass, VerifiedWebhook, WebhookVerificationError,
};
use serde_json::{Value, json};

/// Failure mode a [`FakeConnector`] plays back.
///
/// The four transient scenarios fail on the first call and succeed afterwards,
/// so a test can prove a handler retries rather than giving up.
/// [`FakeConnectorScenario::Exhausted`] never succeeds, which is how a test
/// reaches the attempt cap on purpose.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FakeConnectorScenario {
    /// Every call returns an empty page.
    #[default]
    Healthy,
    /// The first call is rate limited with a 30 second delay.
    RateLimited,
    /// The first call finds the provider unavailable.
    Unavailable,
    /// The first call times out.
    Timeout,
    /// The first call reports the credential as revoked.
    Revoked,
    /// Every call fails as unavailable, so attempts run out.
    Exhausted,
}

impl FakeConnectorScenario {
    /// Returns the stable label used to select the scenario by name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::RateLimited => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Revoked => "revoked",
            Self::Exhausted => "exhausted",
        }
    }
}

impl FromStr for FakeConnectorScenario {
    type Err = UnknownScenario;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "healthy" => Ok(Self::Healthy),
            "rate_limited" => Ok(Self::RateLimited),
            "unavailable" => Ok(Self::Unavailable),
            "timeout" => Ok(Self::Timeout),
            "revoked" => Ok(Self::Revoked),
            "exhausted" => Ok(Self::Exhausted),
            _ => Err(UnknownScenario),
        }
    }
}

/// The scenario name did not match any [`FakeConnectorScenario`].
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
#[error("unknown fake connector scenario")]
pub struct UnknownScenario;

/// Header a [`FakeConnector`] reads the webhook signature from.
pub const FAKE_SIGNATURE_HEADER: &str = "x-baukit-signature";

/// One outcome a [`FakeConnector`] can return from `fetch_page`.
pub type FakeConnectorResponse = Result<ConnectorPage<Value>, ConnectorError>;

/// A scripted connector that plays back failure modes without a network.
///
/// Records are `serde_json::Value`, so a test does not need a normalized type
/// to exercise a handler's retry and health behavior.
///
/// Queued responses set with [`push_response`](Self::push_response) take
/// precedence over the scenario, in order, so a test can script an exact
/// sequence when the scenarios do not fit.
///
/// # Example
///
/// ```rust
/// use baukit_integrations::IntegrationConnector;
/// use baukit_test::{FakeConnector, FakeConnectorScenario};
///
/// # tokio_test_block(async {
/// let connector = FakeConnector::new("strava", FakeConnectorScenario::RateLimited);
/// let job = FakeConnector::job("strava");
///
/// let first = connector.fetch_page(&job, None).await;
/// assert!(first.expect_err("rate limited").is_retryable());
/// assert!(connector.fetch_page(&job, None).await.is_ok());
/// assert_eq!(connector.calls(), 2);
/// # });
/// # fn tokio_test_block<F: std::future::Future>(future: F) -> F::Output {
/// #     tokio::runtime::Builder::new_current_thread()
/// #         .build()
/// #         .expect("runtime")
/// #         .block_on(future)
/// # }
/// ```
#[derive(Clone)]
pub struct FakeConnector {
    provider_id: Arc<str>,
    secret: Arc<Vec<u8>>,
    scenario: FakeConnectorScenario,
    calls: Arc<AtomicUsize>,
    scripted: Arc<Mutex<VecDeque<FakeConnectorResponse>>>,
}

impl FakeConnector {
    /// Builds a connector for `provider_id` playing back `scenario`.
    pub fn new(provider_id: impl Into<String>, scenario: FakeConnectorScenario) -> Self {
        Self {
            provider_id: Arc::from(provider_id.into()),
            secret: Arc::new(Vec::new()),
            scenario,
            calls: Arc::new(AtomicUsize::new(0)),
            scripted: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Builds a connector that verifies webhook signatures against `secret`.
    ///
    /// [`sign`](Self::sign) produces the matching value. It is a plain
    /// non-cryptographic digest, chosen so the fake needs no MAC dependency.
    /// It proves a handler rejects bad signatures; it is not a security
    /// primitive and must never guard anything real.
    pub fn with_webhook_secret(
        provider_id: impl Into<String>,
        scenario: FakeConnectorScenario,
        secret: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            secret: Arc::new(secret.into()),
            ..Self::new(provider_id, scenario)
        }
    }

    /// Returns the signature a valid delivery of `body` carries.
    pub fn sign(&self, body: &[u8]) -> String {
        let mut material = self.secret.as_ref().clone();
        material.extend_from_slice(body);
        let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in material {
            digest ^= u64::from(byte);
            digest = digest.wrapping_mul(0x100_0000_01b3);
        }
        format!("fake={digest:016x}")
    }

    /// Queues one response to return before the scenario resumes.
    pub fn push_response(&self, response: FakeConnectorResponse) {
        self.lock_scripted().push_back(response);
    }

    /// Returns how many times [`fetch_page`](IntegrationConnector::fetch_page)
    /// has been called.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Builds a claimed job for `provider`, for tests that do not have one.
    pub fn job(provider: impl Into<String>) -> ClaimedConnectorJob {
        ClaimedConnectorJob {
            id: uuid::Uuid::now_v7(),
            connection_id: uuid::Uuid::now_v7(),
            owner_id: uuid::Uuid::now_v7(),
            provider: provider.into(),
            job_type: "fake_import".to_owned(),
            payload: json!({}),
            attempts: 0,
            worker_id: "fake-worker".to_owned(),
        }
    }

    fn lock_scripted(&self) -> std::sync::MutexGuard<'_, VecDeque<FakeConnectorResponse>> {
        self.scripted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn next_response(&self) -> FakeConnectorResponse {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(scripted) = self.lock_scripted().pop_front() {
            return scripted;
        }
        if self.scenario == FakeConnectorScenario::Exhausted {
            return Err(unavailable());
        }
        if call > 0 {
            return Ok(ConnectorPage::default());
        }
        match self.scenario {
            FakeConnectorScenario::Healthy => Ok(ConnectorPage::default()),
            FakeConnectorScenario::RateLimited => Err(ConnectorError::new(
                "rate_limited",
                RetryClass::RetryAfter(Duration::from_secs(30)),
            )),
            FakeConnectorScenario::Unavailable | FakeConnectorScenario::Exhausted => {
                Err(unavailable())
            }
            FakeConnectorScenario::Timeout => {
                Err(ConnectorError::new("timeout", RetryClass::Timeout))
            }
            FakeConnectorScenario::Revoked => Err(ConnectorError::revoked("token_revoked")),
        }
    }
}

fn unavailable() -> ConnectorError {
    ConnectorError::new("provider_unavailable", RetryClass::Unavailable)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeWebhookBody {
    delivery_id: String,
    connection_id: uuid::Uuid,
    #[serde(default)]
    next_cursor: Option<Value>,
}

impl IntegrationConnector for FakeConnector {
    type Record = Value;

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<VerifiedWebhook, WebhookVerificationError> {
        let supplied = headers
            .get(FAKE_SIGNATURE_HEADER)
            .ok_or(WebhookVerificationError::InvalidSignature)?;
        if supplied != &self.sign(body) {
            return Err(WebhookVerificationError::InvalidSignature);
        }

        let parsed: FakeWebhookBody =
            serde_json::from_slice(body).map_err(|_| WebhookVerificationError::InvalidEvent)?;
        if parsed.delivery_id.trim().is_empty() {
            return Err(WebhookVerificationError::InvalidEvent);
        }

        Ok(VerifiedWebhook {
            provider: self.provider_id.to_string(),
            signature_digest: supplied.as_bytes().to_vec(),
            delivery_id: parsed.delivery_id,
            connection_id: parsed.connection_id,
            job_kind: "fake_import".to_owned(),
            job_payload: json!({
                "connection_id": parsed.connection_id,
                "next_cursor": parsed.next_cursor,
            }),
            event_metadata: json!({}),
        })
    }

    fn requires_credentials(&self) -> bool {
        false
    }

    fn fetch_page<'a>(
        &'a self,
        job: &'a ClaimedConnectorJob,
        _credentials: Option<&'a [u8]>,
    ) -> ConnectorFuture<'a, Self::Record> {
        Box::pin(async move {
            let mut page = self.next_response()?;
            if page.next_cursor.is_none() {
                page.next_cursor = job
                    .payload
                    .get("next_cursor")
                    .filter(|cursor| !cursor.is_null())
                    .cloned();
            }
            Ok(page)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> ClaimedConnectorJob {
        FakeConnector::job("strava")
    }

    #[tokio::test]
    async fn the_healthy_scenario_always_succeeds() {
        let connector = FakeConnector::new("strava", FakeConnectorScenario::Healthy);
        for _ in 0..3 {
            assert!(connector.fetch_page(&job(), None).await.is_ok());
        }
        assert_eq!(connector.calls(), 3);
    }

    #[tokio::test]
    async fn a_rate_limit_names_its_delay_then_recovers() {
        let connector = FakeConnector::new("strava", FakeConnectorScenario::RateLimited);
        let error = connector
            .fetch_page(&job(), None)
            .await
            .expect_err("first call is rate limited");

        assert_eq!(error.code, "rate_limited");
        assert_eq!(error.retry_after(), Some(Duration::from_secs(30)));
        assert!(error.is_retryable());
        assert!(connector.fetch_page(&job(), None).await.is_ok());
    }

    #[tokio::test]
    async fn transient_failures_recover_on_the_second_attempt() {
        let cases = [
            (
                FakeConnectorScenario::Unavailable,
                "provider_unavailable",
                RetryClass::Unavailable,
            ),
            (
                FakeConnectorScenario::Timeout,
                "timeout",
                RetryClass::Timeout,
            ),
        ];

        for (scenario, code, class) in cases {
            let connector = FakeConnector::new("fitbit", scenario);
            let error = connector
                .fetch_page(&job(), None)
                .await
                .expect_err("first call fails");

            assert_eq!(error.code, code);
            assert_eq!(error.class, class);
            assert!(error.is_retryable());
            assert!(connector.fetch_page(&job(), None).await.is_ok());
        }
    }

    #[tokio::test]
    async fn a_revoked_credential_is_not_retryable() {
        let connector = FakeConnector::new("whoop", FakeConnectorScenario::Revoked);
        let error = connector
            .fetch_page(&job(), None)
            .await
            .expect_err("first call is revoked");

        assert_eq!(error.class, RetryClass::Revoked);
        assert!(!error.is_retryable());
        assert!(connector.fetch_page(&job(), None).await.is_ok());
    }

    #[tokio::test]
    async fn the_exhausted_scenario_never_recovers() {
        let connector = FakeConnector::new("polar", FakeConnectorScenario::Exhausted);
        for _ in 0..5 {
            let error = connector
                .fetch_page(&job(), None)
                .await
                .expect_err("every call fails");
            assert_eq!(error.class, RetryClass::Unavailable);
        }
        assert_eq!(connector.calls(), 5);
    }

    #[tokio::test]
    async fn scripted_responses_run_before_the_scenario() {
        let connector = FakeConnector::new("oura", FakeConnectorScenario::Healthy);
        connector.push_response(Err(ConnectorError::permanent("invalid_payload")));

        let error = connector
            .fetch_page(&job(), None)
            .await
            .expect_err("scripted failure");
        assert_eq!(error.code, "invalid_payload");
        assert!(!error.is_retryable());
        assert!(connector.fetch_page(&job(), None).await.is_ok());
    }

    #[tokio::test]
    async fn a_page_carries_the_cursor_the_job_payload_named() {
        let connector = FakeConnector::new("withings", FakeConnectorScenario::Healthy);
        let mut job = job();
        job.payload = json!({"next_cursor": {"after": "abc"}});

        let page = connector
            .fetch_page(&job, None)
            .await
            .expect("healthy scenario succeeds");
        assert!(page.has_more());
        assert_eq!(page.next_cursor, Some(json!({"after": "abc"})));
    }

    #[test]
    fn a_valid_signature_yields_a_deduplicable_delivery() {
        let connector = FakeConnector::with_webhook_secret(
            "strava",
            FakeConnectorScenario::Healthy,
            *b"secret",
        );
        let connection_id = uuid::Uuid::now_v7();
        let body = serde_json::to_vec(&json!({
            "delivery_id": "delivery-1",
            "connection_id": connection_id,
        }))
        .expect("serializes");
        let headers = BTreeMap::from([(FAKE_SIGNATURE_HEADER.to_owned(), connector.sign(&body))]);

        let webhook = connector
            .verify_webhook(&headers, &body)
            .expect("signature verifies");
        assert_eq!(webhook.delivery_id, "delivery-1");
        assert_eq!(webhook.connection_id, connection_id);
        assert_eq!(webhook.provider, "strava");
        assert!(!webhook.signature_digest.is_empty());
    }

    #[test]
    fn a_bad_signature_is_rejected_before_the_body_is_parsed() {
        let connector = FakeConnector::with_webhook_secret(
            "strava",
            FakeConnectorScenario::Healthy,
            *b"secret",
        );
        let headers = BTreeMap::from([(
            FAKE_SIGNATURE_HEADER.to_owned(),
            "fake=0000000000000000".to_owned(),
        )]);

        assert_eq!(
            connector
                .verify_webhook(&headers, b"not-json")
                .expect_err("verification fails"),
            WebhookVerificationError::InvalidSignature
        );
        assert_eq!(
            connector
                .verify_webhook(&BTreeMap::new(), b"{}")
                .expect_err("verification fails"),
            WebhookVerificationError::InvalidSignature
        );
    }

    #[test]
    fn a_signed_but_unusable_payload_is_an_invalid_event() {
        let connector = FakeConnector::with_webhook_secret(
            "strava",
            FakeConnectorScenario::Healthy,
            *b"secret",
        );
        let body = serde_json::to_vec(&json!({
            "delivery_id": "  ",
            "connection_id": uuid::Uuid::now_v7(),
        }))
        .expect("serializes");
        let headers = BTreeMap::from([(FAKE_SIGNATURE_HEADER.to_owned(), connector.sign(&body))]);

        assert_eq!(
            connector
                .verify_webhook(&headers, &body)
                .expect_err("verification fails"),
            WebhookVerificationError::InvalidEvent
        );
    }

    #[test]
    fn scenarios_round_trip_through_their_labels() {
        for scenario in [
            FakeConnectorScenario::Healthy,
            FakeConnectorScenario::RateLimited,
            FakeConnectorScenario::Unavailable,
            FakeConnectorScenario::Timeout,
            FakeConnectorScenario::Revoked,
            FakeConnectorScenario::Exhausted,
        ] {
            assert_eq!(scenario.as_str().parse(), Ok(scenario));
        }
        assert_eq!(
            " RATE_LIMITED ".parse::<FakeConnectorScenario>(),
            Ok(FakeConnectorScenario::RateLimited)
        );
        assert_eq!(
            "nope".parse::<FakeConnectorScenario>(),
            Err(UnknownScenario)
        );
    }
}
