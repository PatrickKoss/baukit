//! The connector port and the values it exchanges with a job runner.

use std::{collections::BTreeMap, future::Future, pin::Pin};

use baukit_http::RetryClass;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// One unit of import work a runner has leased and is about to execute.
///
/// The payload is opaque here. It is whatever the product enqueued, usually a
/// window of time plus the cursor the previous page returned.
#[derive(Clone, Debug)]
pub struct ClaimedConnectorJob {
    /// Identifier of the leased job.
    pub id: Uuid,
    /// Connection the job imports for.
    pub connection_id: Uuid,
    /// Owner the connection belongs to.
    pub owner_id: Uuid,
    /// Provider the connection talks to, as [`IntegrationConnector::provider_id`].
    pub provider: String,
    /// Product job type, matching the `job_type` the runner dispatched on.
    pub job_type: String,
    /// Product-defined payload, including any cursor from the previous page.
    pub payload: Value,
    /// Attempts already spent on this job, starting at zero for the first try.
    pub attempts: u32,
    /// Worker holding the lease.
    pub worker_id: String,
}

/// One cursor-paged window of external records.
///
/// `T` is the product's normalized record type. The contract never inspects it;
/// it only carries the batch and the cursor that reaches the next window.
#[derive(Clone, Debug)]
pub struct ConnectorPage<T> {
    /// Records this window returned, already normalized by the connector.
    pub records: Vec<T>,
    /// Cursor for the next window, or `None` when the window is complete.
    ///
    /// The shape is the provider's. A runner passes it back through
    /// [`ClaimedConnectorJob::payload`] without reading it.
    pub next_cursor: Option<Value>,
}

impl<T> ConnectorPage<T> {
    /// Builds a page of `records` with no further window to fetch.
    pub fn last(records: Vec<T>) -> Self {
        Self {
            records,
            next_cursor: None,
        }
    }

    /// Builds a page of `records` followed by the window at `cursor`.
    pub fn with_cursor(records: Vec<T>, cursor: Value) -> Self {
        Self {
            records,
            next_cursor: Some(cursor),
        }
    }

    /// Returns whether another window follows this one.
    pub const fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

impl<T> Default for ConnectorPage<T> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            next_cursor: None,
        }
    }
}

/// A failed connector call, classified for the runner.
///
/// The `code` is stable diagnostic text safe to persist and to expose in an
/// operations surface. It is never a localized message and never provider body
/// text, which may carry credentials.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("connector call failed: {code} ({class:?})")]
pub struct ConnectorError {
    /// Stable, non-localized failure code, such as `"rate_limited"`.
    pub code: &'static str,
    /// How the runner should react.
    pub class: RetryClass,
}

impl ConnectorError {
    /// Builds an error from an already classified failure.
    ///
    /// Prefer this over the variant constructors when the class came from
    /// [`baukit_http::classify_http_status`] or
    /// [`baukit_http::classify_transport_error`].
    pub const fn new(code: &'static str, class: RetryClass) -> Self {
        Self { code, class }
    }

    /// Builds an error the credential owner must resolve by reconnecting.
    pub const fn revoked(code: &'static str) -> Self {
        Self::new(code, RetryClass::Revoked)
    }

    /// Builds an error that will fail the same way on every attempt.
    pub const fn permanent(code: &'static str) -> Self {
        Self::new(code, RetryClass::Permanent)
    }

    /// Returns whether another attempt at the same call can succeed.
    pub const fn is_retryable(&self) -> bool {
        self.class.is_retryable()
    }

    /// Returns the wait the provider asked for, if it named one.
    pub const fn retry_after(&self) -> Option<std::time::Duration> {
        self.class.retry_after()
    }
}

/// Why a webhook delivery was rejected before it reached the job store.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum WebhookVerificationError {
    /// The signature was missing, malformed, or did not match the body.
    #[error("webhook signature is missing or invalid")]
    InvalidSignature,
    /// The signature was valid but the payload it covered was not usable.
    #[error("verified webhook content is invalid")]
    InvalidEvent,
}

/// A webhook delivery whose signature verified, reduced to a dedupe identity
/// and the work it should enqueue.
///
/// Verification and the identity are the connector's job. Deciding whether an
/// identity has been seen belongs to the product's delivery store, which is why
/// this type carries the identity rather than a verdict.
#[derive(Clone, Debug)]
pub struct VerifiedWebhook {
    /// Provider that sent the delivery, matching
    /// [`IntegrationConnector::provider_id`].
    pub provider: String,
    /// Provider's own delivery identifier, the primary dedupe key.
    pub delivery_id: String,
    /// Digest of the verified signature, the secondary dedupe key for
    /// providers that reuse or omit delivery identifiers.
    pub signature_digest: Vec<u8>,
    /// Connection the delivery concerns, resolved from the payload.
    pub connection_id: Uuid,
    /// Job type to enqueue for this delivery.
    pub job_kind: String,
    /// Payload for the enqueued job.
    pub job_payload: Value,
    /// Non-sensitive delivery detail worth persisting for operators.
    pub event_metadata: Value,
}

/// Whether a verified delivery produced new work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebhookIngestResult {
    /// The delivery was new and its job was enqueued.
    Accepted,
    /// The delivery repeated one already recorded; no job was enqueued.
    Duplicate,
}

impl WebhookIngestResult {
    /// Returns the stable label for metrics and operations output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Duplicate => "duplicate",
        }
    }
}

/// The future returned by [`IntegrationConnector::fetch_page`].
pub type ConnectorFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<ConnectorPage<T>, ConnectorError>> + Send + 'a>>;

/// Outbound port for importing records from one external provider.
///
/// The trait is the whole generic seam. Everything a provider knows, including
/// OAuth, scopes, response models, cursor encoding, and which record duplicates
/// which, stays inside the implementation.
///
/// Implementations are held behind `Arc<dyn IntegrationConnector<Record = _>>`
/// in a per-provider registry, so the trait stays object safe and returns boxed
/// futures rather than using `async fn`.
pub trait IntegrationConnector: Send + Sync {
    /// Normalized record type this connector produces.
    type Record;

    /// Returns the stable provider identifier, such as `"strava"`.
    ///
    /// It keys the registry, appears in job rows, and labels metrics, so it
    /// must not change once connections exist.
    fn provider_id(&self) -> &str;

    /// Verifies a webhook delivery and reduces it to enqueueable work.
    ///
    /// Check the signature before parsing the body. A caller that reaches
    /// [`WebhookVerificationError::InvalidEvent`] has already proven the sender
    /// holds the shared secret, which is worth distinguishing in metrics.
    ///
    /// Header names in `headers` are lowercase.
    fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<VerifiedWebhook, WebhookVerificationError>;

    /// Returns whether [`fetch_page`](Self::fetch_page) needs a credential.
    ///
    /// Connectors reading a public or pre-shared endpoint override this to
    /// `false` so the runner skips the vault lookup.
    fn requires_credentials(&self) -> bool {
        true
    }

    /// Fetches one window of records for a leased job.
    ///
    /// `credentials` is the plaintext secret material for the connection, or
    /// `None` when [`requires_credentials`](Self::requires_credentials) is
    /// `false`. Read the cursor from `job.payload` and return the next one in
    /// [`ConnectorPage::next_cursor`].
    fn fetch_page<'a>(
        &'a self,
        job: &'a ClaimedConnectorJob,
        credentials: Option<&'a [u8]>,
    ) -> ConnectorFuture<'a, Self::Record>;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn a_page_reports_whether_another_window_follows() {
        let last = ConnectorPage::<u8>::last(vec![1, 2]);
        assert!(!last.has_more());

        let more = ConnectorPage::with_cursor(vec![3], json!({"after": "abc"}));
        assert!(more.has_more());
        assert_eq!(more.next_cursor, Some(json!({"after": "abc"})));
    }

    #[test]
    fn an_empty_page_is_the_default() {
        let page = ConnectorPage::<u8>::default();
        assert!(page.records.is_empty());
        assert!(!page.has_more());
    }

    #[test]
    fn retry_answers_come_from_the_shared_class() {
        let limited = ConnectorError::new(
            "rate_limited",
            RetryClass::RetryAfter(Duration::from_secs(30)),
        );
        assert!(limited.is_retryable());
        assert_eq!(limited.retry_after(), Some(Duration::from_secs(30)));

        assert!(!ConnectorError::revoked("revoked").is_retryable());
        assert!(!ConnectorError::permanent("invalid_payload").is_retryable());
        assert_eq!(
            ConnectorError::permanent("invalid_payload").retry_after(),
            None
        );
    }

    #[test]
    fn ingest_results_have_stable_metric_labels() {
        assert_eq!(WebhookIngestResult::Accepted.as_str(), "accepted");
        assert_eq!(WebhookIngestResult::Duplicate.as_str(), "duplicate");
    }
}
