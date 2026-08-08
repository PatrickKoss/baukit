use std::ops::{Deref, DerefMut};

use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
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
pub struct ApiPath<T>(
    /// The successfully deserialized route parameters.
    pub T,
);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(ApiError::from)
    }
}

impl<T> Deref for ApiPath<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for ApiPath<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A query-string extractor whose rejections use Baukit's standard error envelope.
///
/// This delegates parsing to Axum's [`Query`] extractor and maps every rejection
/// to a safe `validation_failed` [`ApiError`] containing the current request ID.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApiQuery<T>(
    /// The successfully deserialized query parameters.
    pub T,
);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(ApiError::from)
    }
}

impl<T> Deref for ApiQuery<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for ApiQuery<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
