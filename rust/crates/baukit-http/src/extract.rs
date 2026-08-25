use std::ops::{Deref, DerefMut};

use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path as AxumPath, Request},
    http::request::Parts,
};
use serde::de::DeserializeOwned;

use crate::ApiError;

#[derive(Clone, Debug)]
pub(crate) struct JsonRejectionCode(pub(crate) String);

/// A JSON body extractor whose rejections use Baukit's standard error envelope.
///
/// This delegates parsing to Axum's [`Json`] extractor and maps every rejection
/// to a safe `validation_failed` [`ApiError`] containing the current request ID.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApiJson<T>(
    /// The successfully deserialized request body.
    pub T,
);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let rejection_code = request
            .extensions()
            .get::<JsonRejectionCode>()
            .map_or("validation_failed", |code| code.0.as_str())
            .to_owned();
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|_| ApiError::json_rejection(rejection_code))
    }
}

impl<T> Deref for ApiJson<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for ApiJson<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A route-path extractor whose rejections use Baukit's standard error envelope.
///
/// This delegates parsing to Axum's [`Path`] extractor and maps every rejection
/// to a safe `validation_failed` [`ApiError`] containing the current request ID.
#[derive(Clone, Copy, Debug, Default)]
pub struct Path<T>(
    /// The successfully deserialized route parameters.
    pub T,
);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        AxumPath::<T>::from_request_parts(parts, state)
            .await
            .map(|AxumPath(value)| Self(value))
            .map_err(ApiError::from)
    }
}

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Backward-compatible name for [`Path`].
pub use Path as ApiPath;

/// A query-string extractor whose rejections use Baukit's standard error envelope.
///
/// Collection fields use HTML-form repeated-parameter encoding, for example
/// `?tag=rust&tag=typescript` deserializes into `tag: Vec<String>`. This is also
/// OpenAPI's default `form`/`explode=true` query-array representation.
///
/// The extractor intentionally uses the name `Query`, which Utoipa's Axum
/// integration recognizes when inferring query rather than path parameters.
#[derive(Clone, Copy, Debug, Default)]
pub struct Query<T>(
    /// The successfully deserialized query parameters.
    pub T,
);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        serde_html_form::from_str(parts.uri.query().unwrap_or_default())
            .map(Self)
            .map_err(|_| ApiError::query_rejection())
    }
}

impl<T> Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Backward-compatible name for [`Query`].
pub use Query as ApiQuery;
