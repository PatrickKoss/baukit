use std::{error::Error, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::VerificationError;
use baukit_config::{BaukitConfig, ConfigLoader, Environment};
use baukit_test::{KeycloakLogin, KeycloakTestContainer};
use serde_json::Value;

use {{ context.app_crate }}_bin::{AuthProvider, ProductConfig, auth_verifier};

const PRODUCT: &str = "{{ context.app_name }}";

fn product_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn token_claims(token: &str) -> Result<Value, Box<dyn Error>> {
    let payload = token.split('.').nth(1).ok_or("token has no payload")?;
    Ok(serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?)
}

#[tokio::test]
#[ignore = "requires Docker Keycloak"]
async fn local_realm_issues_{{ context.auth_provider }}_shaped_tokens_for_the_production_verifier()
-> Result<(), Box<dyn Error>> {
    let root = product_root();
    let mut realm: Value =
        serde_json::from_slice(&std::fs::read(root.join("keycloak/realm.json"))?)?;
    realm
        .as_object_mut()
        .ok_or("realm export is not an object")?
        .remove("loginTheme");
    let keycloak = KeycloakTestContainer::start(PRODUCT, serde_json::to_vec(&realm)?).await?;
    let config: BaukitConfig<ProductConfig> =
        ConfigLoader::new("local-realm-test", Environment::Local)?
            .local_file(root.join("config/local.toml"))
            .without_dotenv()
            .load()?;
    let mut auth = config.product.auth;
    assert_eq!(auth.provider, AuthProvider::{{ "Clerk" if context.auth_clerk else "Workos" }});
    assert!(auth.issuer.ends_with(&format!("/realms/{PRODUCT}")));
    auth.issuer = keycloak.issuer();
    auth.jwks_uri = Some(keycloak.jwks_uri());
    let token = keycloak
        .access_token(&KeycloakLogin {
            client_id: &format!("{PRODUCT}-local"),
            redirect_uri: "http://127.0.0.1:{{ context.local_callback_port }}/callback",
            username: "test",
            password: "development-password",
            scope: "openid",
        })
        .await?;
    let claims = token_claims(&token)?;
    assert!(claims.get("aud").is_none(), "{claims}");
    assert!(claims["sub"].is_string());
    assert!(claims["sid"].is_string());
{% if context.auth_clerk %}    assert_eq!(claims["azp"], format!("{PRODUCT}-local"));
    assert_eq!(claims["v"], 2);
    assert_eq!(
        claims["o"],
        serde_json::json!({"id": "org_local", "rol": "admin", "slg": "local"})
    );
    assert!(
        claims[baukit_auth::CLERK_SIGN_IN_TIME_CLAIM].is_u64(),
        "{claims}"
    );
    assert_eq!(
        claims[baukit_auth::CLERK_SIGN_IN_TIME_CLAIM],
        claims["auth_time"]
    );
{% else %}    assert_eq!(claims["client_id"], auth.client_id.as_str());
    assert_eq!(claims["org_id"], "org_local");
    assert_eq!(claims["role"], "admin");
    assert_eq!(
        claims["permissions"],
        serde_json::json!(["items:read", "items:write"])
    );
{% endif %}
    let principal = auth_verifier(&auth).await?.verify(&token).await?;
    assert_eq!(Some(principal.subject()), claims["sub"].as_str());
    assert_eq!(principal.organization(), Some("org_local"));
    assert_eq!(principal.issuer(), Some(keycloak.issuer().as_str()));
{% if context.auth_clerk %}    assert_eq!(
        principal.authenticated_at().map(|at| at.timestamp()),
        claims[baukit_auth::CLERK_SIGN_IN_TIME_CLAIM].as_i64()
    );
{% endif %}
{% if context.auth_clerk %}    let other = baukit_auth::WorkOsVerifier::from_jwks_uri(
        keycloak.issuer(),
        "client_{{ context.app_crate }}_local",
        keycloak.jwks_uri(),
    )?;
    let expected = VerificationError::WrongClientId;
{% else %}    let other = baukit_auth::ClerkVerifier::from_jwks_uri(
        keycloak.issuer(),
        ["http://localhost:{{ context.web_port }}"],
        keycloak.jwks_uri(),
    )?;
    let expected = VerificationError::WrongAuthorizedParty;
{% endif %}    let error = other
        .verify(&token)
        .await
        .expect_err("the other provider's verifier accepted a local token");
    assert_eq!(
        std::mem::discriminant(&error),
        std::mem::discriminant(&expected)
    );
    Ok(())
}
