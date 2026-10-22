use super::{
    PlatformError, PlatformEventHandler, PlatformInbox, PlatformReceiver, StandaloneEntitlements,
    TicketVerifier,
};
use baukit_config::{Secret, Validate, ValidationError, ValidationErrors};
use serde::Deserialize;
use std::sync::Arc;
use url::Url;

const MAX_PRODUCT_ID_BYTES: usize = 64;

/// Embed as `platform` in product settings to load `<PREFIX>__PLATFORM__*`.
/// An absent issuer selects standalone mode. Partial hub settings are invalid.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PlatformConfig {
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub hub_issuer: Option<String>,
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub jwks_url: Option<String>,
    pub product_id: String,
    pub receiver_secret: Option<Secret<String>>,
    pub previous_receiver_secret: Option<Secret<String>>,
    pub standalone_default: StandaloneEntitlements,
}
impl Validate for PlatformConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        let mut invalid = |field, message| errors.push(ValidationError::new(field, message));
        if self.standalone_default.tier.trim().is_empty() {
            invalid("platform.standalone_default.tier", "must not be empty");
        }
        if let Some(issuer) = &self.hub_issuer {
            if !http_url(issuer) {
                invalid(
                    "platform.hub_issuer",
                    "must be an HTTP(S) URL without credentials or fragment",
                );
            }
            if !self.jwks_url.as_deref().is_some_and(http_url) {
                invalid(
                    "platform.jwks_url",
                    "must be an HTTP(S) URL without credentials or fragment",
                );
            }
            if self.product_id.is_empty()
                || self.product_id.len() > MAX_PRODUCT_ID_BYTES
                || !self
                    .product_id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                invalid("platform.product_id", "must be a product identifier");
            }
            if !self
                .receiver_secret
                .as_ref()
                .is_some_and(|s| !s.expose().is_empty())
            {
                invalid("platform.receiver_secret", "is required in hub mode");
            }
        } else if self.jwks_url.is_some()
            || self.receiver_secret.is_some()
            || self.previous_receiver_secret.is_some()
        {
            invalid(
                "platform.hub_issuer",
                "is required when hub settings are present",
            );
        }
        if self
            .previous_receiver_secret
            .as_ref()
            .is_some_and(|s| s.expose().is_empty())
        {
            invalid("platform.previous_receiver_secret", "must not be empty");
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}
impl PlatformConfig {
    pub fn standalone(&self) -> bool {
        self.hub_issuer.is_none()
    }
    pub fn entitlements(
        &self,
        cache: Arc<dyn super::EntitlementCache>,
    ) -> Result<super::Entitlements, ValidationErrors> {
        self.validate()?;
        Ok(if self.standalone() {
            super::Entitlements::standalone(
                self.product_id.clone(),
                self.standalone_default.clone(),
            )
        } else {
            super::Entitlements::hub(self.product_id.clone(), cache)
        })
    }
    pub fn receiver(
        &self,
        inbox: Arc<dyn PlatformInbox>,
        handler: Arc<dyn PlatformEventHandler>,
    ) -> Result<Option<PlatformReceiver>, ValidationErrors> {
        self.validate()?;
        self.receiver_secret
            .clone()
            .map(|secret| {
                PlatformReceiver::new(
                    secret,
                    self.previous_receiver_secret.clone(),
                    inbox,
                    handler,
                )
                .map_err(|_| {
                    ValidationErrors::new(vec![ValidationError::new(
                        "platform.receiver_secret",
                        "invalid secret",
                    )])
                })
            })
            .transpose()
    }
    /// The caller supplies the guarded client's public, local or private-network policy.
    pub fn ticket_verifier(
        &self,
        client: baukit_egress::GuardedClient,
    ) -> Result<Option<TicketVerifier>, PlatformError> {
        self.validate().map_err(|_| PlatformError::Invalid)?;
        match (&self.hub_issuer, &self.jwks_url) {
            (Some(issuer), Some(jwks)) => {
                TicketVerifier::new(issuer.clone(), self.product_id.clone(), jwks, client)
                    .map(Some)
                    .map_err(|_| PlatformError::Invalid)
            }
            _ => Ok(None),
        }
    }
}
fn http_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}
