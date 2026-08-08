use axum::Router;

use crate::{ApiError, HttpOptions, middleware::layers};

/// Finalizes a public router with standard 404/405 envelopes and HTTP layers.
///
/// Call this once after defining and merging all product routes. Unmatched paths
/// return `not_found`, unsupported methods return `method_not_allowed`, and both
/// responses include the request ID installed by the lifecycle middleware.
pub fn finalize<S>(router: Router<S>, options: HttpOptions) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    layers(
        router
            .fallback(route_not_found)
            .method_not_allowed_fallback(method_not_allowed),
        options,
    )
}

async fn route_not_found() -> ApiError {
    ApiError::not_found("Route not found")
}

async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}
