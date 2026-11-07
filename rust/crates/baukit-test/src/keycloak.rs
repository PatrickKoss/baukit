use std::{fmt, time::Duration};

use aws_lc_rs::digest;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{
    StatusCode,
    header::{COOKIE, LOCATION, SET_COOKIE},
    redirect::Policy,
};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt as _,
    core::{IntoContainerPort as _, WaitFor},
    runners::AsyncRunner,
};
use thiserror::Error;

const KEYCLOAK_IMAGE: &str = "quay.io/keycloak/keycloak";
const KEYCLOAK_TAG: &str = "26.8.0";
const KEYCLOAK_PORT: u16 = 8080;
const IMPORT_PATH: &str = "/opt/keycloak/data/import/realm.json";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// A disposable Keycloak that imported one realm export at startup.
///
/// Keep this value alive while tokens are issued or verified. Dropping it
/// removes the container.
pub struct KeycloakTestContainer {
    base_url: String,
    realm: String,
    _container: ContainerAsync<GenericImage>,
}

/// One user's PKCE authorization-code login against a public client.
///
/// Debug output redacts the password.
#[derive(Clone)]
pub struct KeycloakLogin<'a> {
    /// Public client that has the authorization-code flow and PKCE S256.
    pub client_id: &'a str,
    /// A redirect URI registered on that client. It is never contacted.
    pub redirect_uri: &'a str,
    /// Username of an imported user.
    pub username: &'a str,
    /// That user's password.
    pub password: &'a str,
    /// Space-separated scopes to request.
    pub scope: &'a str,
}

impl fmt::Debug for KeycloakLogin<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeycloakLogin")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("scope", &self.scope)
            .finish()
    }
}

impl KeycloakTestContainer {
    /// Starts Keycloak with `realm_json` imported as realm `realm`.
    pub async fn start(
        realm: impl Into<String>,
        realm_json: impl Into<Vec<u8>>,
    ) -> Result<Self, KeycloakTestError> {
        let container = GenericImage::new(KEYCLOAK_IMAGE, KEYCLOAK_TAG)
            .with_exposed_port(KEYCLOAK_PORT.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Listening on:"))
            .with_cmd(["start-dev", "--import-realm"])
            .with_env_var("KC_HOSTNAME_STRICT", "false")
            .with_copy_to(IMPORT_PATH, realm_json.into())
            .with_labels(crate::container_labels())
            .with_startup_timeout(STARTUP_TIMEOUT)
            .start()
            .await?;
        let port = container.get_host_port_ipv4(KEYCLOAK_PORT.tcp()).await?;
        Ok(Self {
            base_url: format!("http://127.0.0.1:{port}"),
            realm: realm.into(),
            _container: container,
        })
    }

    /// Returns the issuer Keycloak writes into tokens for the imported realm.
    #[must_use]
    pub fn issuer(&self) -> String {
        format!("{}/realms/{}", self.base_url, self.realm)
    }

    /// Returns the realm's signing-key endpoint.
    #[must_use]
    pub fn jwks_uri(&self) -> String {
        format!("{}/protocol/openid-connect/certs", self.issuer())
    }

    /// Signs a user in through the login form and returns the access token.
    pub async fn access_token(
        &self,
        login: &KeycloakLogin<'_>,
    ) -> Result<String, KeycloakTestError> {
        let client = crate::test_http_client()
            .redirect(Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        let verifier = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let challenge =
            URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()));
        let state = uuid::Uuid::new_v4().simple().to_string();
        let endpoint = format!("{}/protocol/openid-connect", self.issuer());
        let authorization = reqwest::Url::parse_with_params(
            &format!("{endpoint}/auth"),
            [
                ("client_id", login.client_id),
                ("redirect_uri", login.redirect_uri),
                ("response_type", "code"),
                ("scope", login.scope),
                ("state", state.as_str()),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ],
        )
        .map_err(|_| KeycloakTestError::Login("authorization URL is invalid"))?;
        let page = client.get(authorization).send().await?;
        let cookies = page
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok()?.split(';').next())
            .collect::<Vec<_>>()
            .join("; ");
        let body = page.text().await?;
        let action = login_form_action(&body)?;
        let callback = client
            .post(action)
            .header(COOKIE, cookies)
            .form(&[
                ("username", login.username),
                ("password", login.password),
                ("credentialId", ""),
            ])
            .send()
            .await?;
        if callback.status() != StatusCode::FOUND {
            return Err(KeycloakTestError::Login("login did not redirect"));
        }
        let location = callback
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(KeycloakTestError::Login("login redirect has no location"))?;
        let location = reqwest::Url::parse(location)
            .map_err(|_| KeycloakTestError::Login("login redirect is not a URL"))?;
        let parameter = |name: &str| {
            location
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        if parameter("state").as_deref() != Some(state.as_str()) {
            return Err(KeycloakTestError::Login("state did not match"));
        }
        let code = parameter("code").ok_or(KeycloakTestError::Login("callback has no code"))?;
        let tokens: serde_json::Value = client
            .post(format!("{endpoint}/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", login.client_id),
                ("redirect_uri", login.redirect_uri),
                ("code", code.as_str()),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        tokens["access_token"]
            .as_str()
            .map(str::to_owned)
            .ok_or(KeycloakTestError::Login(
                "token response has no access token",
            ))
    }
}

fn login_form_action(page: &str) -> Result<String, KeycloakTestError> {
    let form = page
        .find("id=\"kc-form-login\"")
        .and_then(|start| page[..start].rfind("<form").map(|form| &page[form..]))
        .ok_or(KeycloakTestError::Login("login form was not found"))?;
    let action = form
        .split_once("action=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(action, _)| action.replace("&amp;", "&"))
        .ok_or(KeycloakTestError::Login("login form has no action"))?;
    Ok(action)
}

/// Failure to start the Keycloak fixture or sign a user in.
#[derive(Debug, Error)]
pub enum KeycloakTestError {
    /// Testcontainers could not start or inspect Keycloak.
    #[error("could not start Keycloak test container: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// An HTTP request to Keycloak failed.
    #[error("Keycloak request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The login flow did not produce an authorization code or token.
    #[error("Keycloak login failed: {0}")]
    Login(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_form_action_is_unescaped() -> Result<(), KeycloakTestError> {
        let page = r#"<div><form id="kc-form-login" class="x" action="http://kc/a?b=1&amp;c=2" method="post"></form></div>"#;
        assert_eq!(login_form_action(page)?, "http://kc/a?b=1&c=2");
        assert!(login_form_action("<form action=\"x\"></form>").is_err());
        Ok(())
    }

    #[test]
    fn login_debug_redacts_password() {
        let login = KeycloakLogin {
            client_id: "web",
            redirect_uri: "http://localhost/callback",
            username: "alice",
            password: "hunter2-password",
            scope: "openid",
        };

        let debug = format!("{login:?}");

        assert!(!debug.contains("hunter2-password"));
        assert!(debug.contains("alice"));
        assert!(debug.contains("<redacted>"));
    }
}
