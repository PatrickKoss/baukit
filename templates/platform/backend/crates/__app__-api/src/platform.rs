use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderValue, header};
use baukit_config::{Secret, ValidationError, ValidationErrors};
use baukit_egress::{AddressPolicy, EgressError, EgressOptions, EgressRequest, GuardedClient};
use baukit_suite::platform::{
    EntitlementSnapshot, EntitlementSource, EntitlementSourceError, PlatformConfig, TicketVerifier,
};
use serde::Deserialize;
use url::Url;

const RESPONSE_LIMIT: usize = 64 * 1024;
/// Retry hint when the hub client sheds load without naming a delay.
const SATURATED_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PlatformHubConfig {
    pub ticket_url: Option<String>,
    pub service_token: Option<Secret<String>>,
    pub allow_local_http: bool,
}

impl PlatformHubConfig {
    pub fn validate_for(&self, platform: &PlatformConfig) -> Result<(), ValidationErrors> {
        let valid = if platform.standalone() {
            self.ticket_url.is_none() && self.service_token.is_none() && !self.allow_local_http
        } else {
            self.ticket_url.as_deref().is_some_and(|value| {
                Url::parse(value).is_ok_and(|url| {
                    (url.scheme() == "https" || self.allow_local_http && url.scheme() == "http")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                })
            }) && self.service_token.as_ref().is_some_and(|token| {
                !token.expose().is_empty()
                    && HeaderValue::from_str(&format!("Bearer {}", token.expose())).is_ok()
            })
        };
        if valid {
            Ok(())
        } else {
            Err(ValidationErrors::new(vec![ValidationError::new(
                "platform_hub",
                "hub mode requires a ticket URL and service token; standalone mode requires neither",
            )]))
        }
    }
}

/// POSTs {suiteSubject, productId} and verifies the returned {ticket} with hub JWKS.
pub struct HubTicketSource {
    client: GuardedClient,
    url: Url,
    headers: HeaderMap,
    verifier: TicketVerifier,
}

impl HubTicketSource {
    pub fn new(
        platform: &PlatformConfig,
        hub: &PlatformHubConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        hub.validate_for(platform)?;
        let url = Url::parse(
            hub.ticket_url
                .as_deref()
                .ok_or("hub ticket URL is required")?,
        )?;
        let policy = if hub.allow_local_http {
            AddressPolicy::AllowLoopback
        } else {
            AddressPolicy::PublicOnly
        };
        let client = GuardedClient::new(
            "hub",
            EgressOptions::default()
                .with_policy(policy)
                .with_max_response_bytes(RESPONSE_LIMIT)?
                .with_error_body_limit(RESPONSE_LIMIT)?,
        )?;
        let verifier = platform
            .ticket_verifier(client.clone())?
            .ok_or("hub mode is required")?;
        let token = hub
            .service_token
            .as_ref()
            .ok_or("hub service token is required")?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", token.expose()))?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, authorization);
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        Ok(Self {
            client,
            url,
            headers,
            verifier,
        })
    }
}

#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

#[async_trait]
impl EntitlementSource for HubTicketSource {
    async fn pull(
        &self,
        subject: &str,
        product_id: &str,
    ) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "suiteSubject": subject,
            "productId": product_id,
        }))
        .map_err(|_| EntitlementSourceError::Invalid)?;
        let response = self
            .client
            .execute(
                EgressRequest::post(self.url.clone())
                    .with_headers(self.headers.clone())
                    .with_body(body),
            )
            .await
            .map_err(|error| source_error(&error))?;
        let response: TicketResponse =
            serde_json::from_slice(response.body()).map_err(|_| EntitlementSourceError::Invalid)?;
        let claims = self
            .verifier
            .verify(&response.ticket)
            .await
            .map_err(|_| EntitlementSourceError::Unavailable)?;
        if claims.sub != subject || claims.aud != product_id {
            return Err(EntitlementSourceError::Invalid);
        }
        Ok(claims.into())
    }
}

/// Maps a failed hub call. An open breaker or a full bulkhead cools the
/// entitlement path down instead of reporting the hub as broken.
fn source_error(error: &EgressError) -> EntitlementSourceError {
    match error {
        EgressError::CircuitOpen { retry_after } => EntitlementSourceError::CoolingDown {
            retry_after: *retry_after,
        },
        EgressError::Saturated => EntitlementSourceError::CoolingDown {
            retry_after: error
                .retry_class()
                .retry_after()
                .unwrap_or(SATURATED_RETRY_AFTER),
        },
        EgressError::Status {
            status,
            body: Some(body),
            ..
        } => EntitlementSourceError::from_hub_response(
            status.as_u16(),
            body.problem_code().unwrap_or(""),
        ),
        _ => EntitlementSourceError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use baukit_egress::EgressError;
    use baukit_suite::platform::EntitlementSourceError;

    use super::source_error;

    #[test]
    fn an_open_circuit_cools_the_source_down() {
        let retry_after = Duration::from_secs(9);
        assert!(matches!(
            source_error(&EgressError::CircuitOpen { retry_after }),
            EntitlementSourceError::CoolingDown { retry_after: delay } if delay == retry_after
        ));
    }

    #[test]
    fn a_saturated_client_cools_the_source_down() {
        assert!(matches!(
            source_error(&EgressError::Saturated),
            EntitlementSourceError::CoolingDown { retry_after } if !retry_after.is_zero()
        ));
    }
}
