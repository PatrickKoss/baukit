use crate::domain::{
    PeerRegistry, PeerRegistrySettings, PeerUrls, SUITE_INITIAL_REPLAY_DAYS, SUITE_MAX_REPLAY_DAYS,
    SuiteDataError,
};
use baukit_config::{Validate, ValidationError, ValidationErrors};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

/// Put this section in the product settings as `suite` to load `<PREFIX>__SUITE__*`.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SuiteConfig {
    #[serde(deserialize_with = "empty_string_as_none")]
    pub public_api_url: Option<String>,
    #[serde(deserialize_with = "empty_string_as_none")]
    pub public_web_url: Option<String>,
    pub peers: BTreeMap<String, PeerUrls>,
    #[serde(deserialize_with = "flexible_bool")]
    pub allow_loopback: bool,
    pub initial_replay_days: u32,
    #[serde(deserialize_with = "flexible_bool")]
    pub share_xp: bool,
    #[serde(deserialize_with = "empty_string_as_none")]
    pub identity_domain: Option<String>,
    pub identity_claim: String,
}
impl Default for SuiteConfig {
    fn default() -> Self {
        Self {
            public_api_url: None,
            public_web_url: None,
            peers: BTreeMap::new(),
            allow_loopback: false,
            initial_replay_days: SUITE_INITIAL_REPLAY_DAYS,
            share_xp: true,
            identity_domain: None,
            identity_claim: "suite_sub".into(),
        }
    }
}
impl SuiteConfig {
    pub fn registry(
        &self,
        own_app: &str,
        peers_json: &str,
    ) -> Result<PeerRegistry, SuiteDataError> {
        PeerRegistry::new(
            own_app,
            peers_json,
            PeerRegistrySettings {
                public_api_url: self.public_api_url.clone(),
                public_web_url: self.public_web_url.clone(),
                peers: self.peers.clone(),
                allow_loopback: self.allow_loopback,
            },
        )
    }
    /// Validate the embedded registry and require encryption whenever suite mode is active.
    #[cfg(feature = "runtime")]
    pub fn validate_for(
        &self,
        own_app: &str,
        peers_json: &str,
        cipher: Option<&baukit_credential_vault::CredentialCipher>,
    ) -> Result<(), ValidationErrors> {
        self.validate()?;
        let registry = self.registry(own_app, peers_json).map_err(|error| {
            ValidationErrors::new(vec![ValidationError::new("suite", error.to_string())])
        })?;
        if !registry.standalone() && cipher.is_none() {
            return Err(ValidationErrors::new(vec![ValidationError::new(
                "suite",
                "Suite mode requires a credential cipher",
            )]));
        }
        Ok(())
    }
}
impl Validate for SuiteConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if self.initial_replay_days > SUITE_MAX_REPLAY_DAYS {
            errors.push(ValidationError::new(
                "suite.initial_replay_days",
                "must be at most 365",
            ));
        }
        if self.identity_claim.is_empty()
            || self.identity_claim.len() > 128
            || self.identity_claim.chars().any(char::is_control)
        {
            errors.push(ValidationError::new(
                "suite.identity_claim",
                "invalid claim",
            ));
        }
        if self.identity_domain.as_ref().is_some_and(|domain| {
            domain.is_empty()
                || domain.len() > 128
                || domain.trim() != domain
                || domain.contains('|')
                || domain.chars().any(char::is_control)
        }) {
            errors.push(ValidationError::new(
                "suite.identity_domain",
                "invalid identity domain",
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}
pub(crate) fn empty_string_as_none<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.filter(|value| !value.is_empty()))
}

fn flexible_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        Bool(bool),
        Text(String),
    }
    match Input::deserialize(deserializer)? {
        Input::Bool(value) => Ok(value),
        Input::Text(value) if value.is_empty() => Ok(false),
        Input::Text(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default, Deserialize)]
    struct Product {
        suite: SuiteConfig,
    }
    impl Validate for Product {
        fn validate(&self) -> Result<(), ValidationErrors> {
            self.suite.validate()
        }
    }
    #[test]
    fn standard_nested_environment_loads_the_suite_section() {
        const CHILD: &str = "BAUKIT_SUITE_CONFIG_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result =
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "config::tests::standard_nested_environment_loads_the_suite_section",
                    ])
                    .env(CHILD, "1")
                    .env(
                        "SUITE_CONFIG_TEST__SUITE__PUBLIC_API_URL",
                        "https://alpha.example/api/v1",
                    )
                    .env(
                        "SUITE_CONFIG_TEST__SUITE__PUBLIC_WEB_URL",
                        "https://alpha.example",
                    )
                    .env(
                        "SUITE_CONFIG_TEST__SUITE__PEERS__BETA__API_URL",
                        "https://beta.example/api/v1",
                    )
                    .env(
                        "SUITE_CONFIG_TEST__SUITE__PEERS__BETA__WEB_URL",
                        "https://beta.example",
                    )
                    .env("SUITE_CONFIG_TEST__SUITE__ALLOW_LOOPBACK", "")
                    .env("SUITE_CONFIG_TEST__SUITE__SHARE_XP", "")
                    .env("SUITE_CONFIG_TEST__SUITE__INITIAL_REPLAY_DAYS", "42")
                    .env("SUITE_CONFIG_TEST__SUITE__IDENTITY_DOMAIN", "shared")
                    .env("SUITE_CONFIG_TEST__SUITE__IDENTITY_CLAIM", "sub")
                    .output()
                    .expect("child config test");
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            return;
        }
        let config = baukit_config::ConfigLoader::new(
            "suite-config-test",
            baukit_config::Environment::Local,
        )
        .expect("loader")
        .without_dotenv()
        .without_local_file()
        .load::<Product>()
        .expect("load");
        let suite = config.product.suite;
        assert_eq!(suite.initial_replay_days, 42);
        assert!(!suite.share_xp);
        assert!(!suite.allow_loopback);
        assert_eq!(suite.identity_domain.as_deref(), Some("shared"));
        assert_eq!(suite.identity_claim, "sub");
        assert!(
            !suite
                .registry(
                    "alpha",
                    include_str!("../../../../fixtures/suite-events/v1/peers.json")
                )
                .expect("registry")
                .standalone()
        );
    }
    #[test]
    fn booleans_accept_empty_strings_and_preserve_boolean_values() {
        for (value, expected) in [
            (serde_json::json!(""), false),
            (serde_json::json!("false"), false),
            (serde_json::json!("true"), true),
            (serde_json::json!(false), false),
            (serde_json::json!(true), true),
        ] {
            let config: SuiteConfig = serde_json::from_value(
                serde_json::json!({"share_xp":value,"allow_loopback":value}),
            )
            .expect("bool");
            assert_eq!(config.share_xp, expected);
            assert_eq!(config.allow_loopback, expected);
        }
        for value in [
            serde_json::json!("yes"),
            serde_json::json!(1),
            serde_json::Value::Null,
        ] {
            assert!(
                serde_json::from_value::<SuiteConfig>(serde_json::json!({"share_xp":value}))
                    .is_err()
            );
        }
        let defaults: SuiteConfig = serde_json::from_str("{}").expect("defaults");
        assert!(defaults.share_xp);
        assert!(!defaults.allow_loopback);
    }
    #[test]
    fn settings_reject_invalid_identity_and_replay_and_require_active_cipher() {
        for days in [366, u32::MAX] {
            assert!(
                SuiteConfig {
                    initial_replay_days: days,
                    ..SuiteConfig::default()
                }
                .validate()
                .is_err()
            );
        }
        for domain in ["padded ", "with|separator", "\n"] {
            assert!(
                SuiteConfig {
                    identity_domain: Some(domain.into()),
                    ..SuiteConfig::default()
                }
                .validate()
                .is_err()
            );
        }
        let empty: SuiteConfig =
            serde_json::from_str(r#"{"public_api_url":"","identity_domain":""}"#)
                .expect("empty optional settings");
        assert!(empty.public_api_url.is_none());
        assert!(empty.identity_domain.is_none());
        let peers = include_str!("../../../../fixtures/suite-events/v1/peers.json");
        assert!(empty.validate_for("alpha", peers, None).is_ok());
        let active = SuiteConfig {
            public_api_url: Some("https://alpha.example/api/v1".into()),
            public_web_url: Some("https://alpha.example".into()),
            peers: BTreeMap::from([(
                "beta".into(),
                PeerUrls {
                    api_url: Some("https://beta.example/api/v1".into()),
                    web_url: Some("https://beta.example".into()),
                },
            )]),
            ..SuiteConfig::default()
        };
        assert!(active.validate_for("alpha", peers, None).is_err());
    }
}
