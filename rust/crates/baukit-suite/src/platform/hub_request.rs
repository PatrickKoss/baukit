use axum::http::HeaderValue;
use serde::Serialize;

use super::HubError;

/// Whether a request may be sent again after a 401 with a fresh token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Replay {
    Allowed,
    Forbidden,
}

/// A `/platform/v1` call for [`super::HubClient::send_json`].
///
/// GETs, [`Self::idempotent`] POSTs and requests with an idempotency key are
/// replayed once after a 401 with a fresh token. A plain [`Self::post`] is
/// never replayed: the hub may already have applied it.
#[derive(Clone, Debug)]
pub struct HubRequest {
    segments: Vec<String>,
    query: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    idempotency_key: Option<HeaderValue>,
    replay: Replay,
}

impl HubRequest {
    /// `GET <api_url>/platform/v1/<segments>`.
    #[must_use]
    pub fn get(segments: &[&str]) -> Self {
        Self::get_with_query(segments, &[])
    }

    /// `GET <api_url>/platform/v1/<segments>?<query>`. An empty `query` sends no `?`.
    #[must_use]
    pub fn get_with_query(segments: &[&str], query: &[(&str, &str)]) -> Self {
        Self {
            segments: owned(segments),
            query: query
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: None,
            idempotency_key: None,
            replay: Replay::Allowed,
        }
    }

    /// A JSON POST that has the same effect when sent twice, such as a read
    /// through POST.
    ///
    /// # Errors
    ///
    /// `HubError::InvalidResponse` when `body` does not serialize.
    pub fn idempotent<B: Serialize>(segments: &[&str], body: &B) -> Result<Self, HubError> {
        Self::post_with(segments, body, Replay::Allowed)
    }

    /// A JSON POST that is not replayed after a 401.
    ///
    /// # Errors
    ///
    /// `HubError::InvalidResponse` when `body` does not serialize.
    pub fn post<B: Serialize>(segments: &[&str], body: &B) -> Result<Self, HubError> {
        Self::post_with(segments, body, Replay::Forbidden)
    }

    /// Sends `key` as `Idempotency-Key`, which makes the request replayable.
    ///
    /// # Errors
    ///
    /// `HubError::InvalidResponse` when `key` is empty or not a valid header value.
    pub fn with_idempotency_key(mut self, key: &str) -> Result<Self, HubError> {
        if key.is_empty() {
            return Err(HubError::InvalidResponse);
        }
        self.idempotency_key =
            Some(HeaderValue::from_str(key).map_err(|_| HubError::InvalidResponse)?);
        self.replay = Replay::Allowed;
        Ok(self)
    }

    fn post_with<B: Serialize>(
        segments: &[&str],
        body: &B,
        replay: Replay,
    ) -> Result<Self, HubError> {
        Ok(Self {
            segments: owned(segments),
            query: Vec::new(),
            body: Some(serde_json::to_vec(body).map_err(|_| HubError::InvalidResponse)?),
            idempotency_key: None,
            replay,
        })
    }

    pub(super) fn replay(&self) -> Replay {
        self.replay
    }

    pub(super) fn segment_refs(&self) -> Vec<&str> {
        self.segments.iter().map(String::as_str).collect()
    }

    pub(super) fn query(&self) -> &[(String, String)] {
        &self.query
    }

    pub(super) fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }

    pub(super) fn idempotency_key(&self) -> Option<&HeaderValue> {
        self.idempotency_key.as_ref()
    }
}

fn owned(segments: &[&str]) -> Vec<String> {
    segments
        .iter()
        .map(|segment| (*segment).to_owned())
        .collect()
}
