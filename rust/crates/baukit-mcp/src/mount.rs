use std::sync::{Arc, OnceLock};

use axum::{
    Json, Router,
    extract::Request,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use baukit_ratelimit::RateLimitStore;
use http::{HeaderValue, StatusCode, header};
use serde_json::json;

use crate::{AuthenticationPolicy, McpConfig, McpConfigError, ToolService};

#[derive(Clone, Default)]
pub(crate) struct ScopeDenial(pub Arc<OnceLock<Vec<String>>>);

/// Mounts authenticated MCP with optional resources and prompts supplied by McpServices.
pub async fn router(
    config: McpConfig,
    tools: Arc<dyn ToolService>,
    store: Arc<dyn RateLimitStore>,
    policy: Arc<dyn AuthenticationPolicy>,
) -> Result<Router, McpConfigError> {
    let enabled = config.enabled;
    let metadata_url = if enabled {
        config.metadata_url()?
    } else {
        String::new()
    };
    let router = crate::security::router(config, tools, store, policy).await?;
    if !enabled {
        return Ok(router);
    }
    Ok(router.layer(middleware::from_fn(
        move |mut request: Request, next: Next| {
            let metadata_url = metadata_url.clone();
            async move {
                let denial = ScopeDenial::default();
                request.extensions_mut().insert(denial.clone());
                let response = next.run(request).await;
                let Some(scopes) = denial.0.get() else {
                    return response;
                };
                scope_denial(&metadata_url, scopes)
            }
        },
    )))
}

fn scope_denial(metadata_url: &str, scopes: &[String]) -> Response {
    let challenge = format!(
        "Bearer resource_metadata=\"{metadata_url}\", error=\"insufficient_scope\", scope=\"{}\"",
        scopes.join(" ")
    );
    match HeaderValue::from_str(&challenge) {
        Ok(challenge) => (
            StatusCode::FORBIDDEN,
            [
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                (header::WWW_AUTHENTICATE, challenge),
            ],
            Json(json!({"error":"insufficient_scope"})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CACHE_CONTROL, "no-store")],
            Json(json!({"error":"invalid_challenge_configuration"})),
        )
            .into_response(),
    }
}
