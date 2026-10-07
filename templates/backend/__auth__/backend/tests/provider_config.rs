use baukit_config::{BaukitConfig, ConfigLoader, Environment};

use std::{error::Error, time::Duration};

use {{ context.app_crate }}_bin::AuthProvider;
use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::auth_verifier;
{% if context.mcp %}use {{ context.app_crate }}_bin::mcp_verifier;
{% endif %}
#[tokio::test]
async fn external_issuer_configuration_selects_the_verifier_end_to_end()
-> Result<(), Box<dyn Error>> {
    let server = baukit_test::MockOidcServer::start().await?;
    for provider in ["oidc", "clerk", "workos"] {
        let file = tempfile::Builder::new().suffix(".toml").tempfile()?;
        let jwks = if provider == "oidc" {
            String::new()
        } else {
            format!("jwks_uri = {:?}\n", server.jwks_url())
        };
        std::fs::write(
            file.path(),
            format!(
                "[auth]\nprovider = {provider:?}\nissuer = {:?}\naudience = \"api\"\nclient_id = \"client_app\"\nauthorized_parties = [\"https://app.example\"]\n{jwks}",
                server.issuer(),
            ),
        )?;
        let config: BaukitConfig<ProductConfig> =
            ConfigLoader::new("provider-test", Environment::Local)?
                .local_file(file.path())
                .without_dotenv()
                .load()?;
        let verifier = auth_verifier(&config.product.auth).await?;
        let claims = server
            .claims("user_123", "api", Duration::from_secs(300))?
            .claim("azp", "https://app.example")
            .claim("client_id", "client_app");
        assert_eq!(
            verifier.verify(&server.mint(&claims)?).await?.subject(),
            "user_123"
        );
        assert!(
            verifier
                .verify(&server.mint(&claims.clone().issuer("https://wrong.example"))?)
                .await
                .is_err()
        );
        if config.product.auth.provider == AuthProvider::Oidc {
            assert!(
                verifier
                    .verify(&server.mint(&claims.audience("wrong"))?)
                    .await
                    .is_err()
            );
        }
    }
    Ok(())
}

{% if context.mcp %}#[tokio::test]
async fn mcp_enforces_provider_token_binding_and_introspection_is_restricted()
-> Result<(), Box<dyn Error>> {
    let server = baukit_test::MockOidcServer::start().await?;
    for provider in [
        AuthProvider::Oidc,
        AuthProvider::Clerk,
        AuthProvider::Workos,
    ] {
        let auth = {{ context.app_crate }}_bin::AuthConfig {
            provider,
            ..Default::default()
        };
        let mut config = baukit_mcp::McpConfig {
            enabled: true,
            issuer: "https://public-issuer.example/tenant/".into(),
            jwks_uri: Some(server.jwks_url().into()),
            oauth_client_id: Some("client_mcp".into()),
            resource_url: "https://mcp.example/mcp".into(),
            allowed_hosts: vec!["mcp.example".into()],
            ..Default::default()
        };
        let file = tempfile::Builder::new().suffix(".toml").tempfile()?;
        let provider_name = match provider {
            AuthProvider::Oidc => "oidc",
            AuthProvider::Clerk => "clerk",
            AuthProvider::Workos => "workos",
        };
        std::fs::write(
            file.path(),
            format!(
                "[auth]\nprovider = {provider_name:?}\nissuer = \"https://rest.example\"\nclient_id = \"client_app\"\n[mcp]\nenabled = true\nissuer = {:?}\njwks_uri = {:?}\noauth_client_id = \"client_mcp\"\nresource_url = {:?}\nallowed_hosts = [\"mcp.example\"]\n",
                config.issuer,
                server.jwks_url(),
                config.resource_url,
            ),
        )?;
        let loaded: BaukitConfig<ProductConfig> =
            ConfigLoader::new("internal-jwks-test", Environment::Local)?
                .local_file(file.path())
                .without_dotenv()
                .load()?;
        assert_eq!(loaded.product.mcp.issuer, config.issuer);
        assert_ne!(server.issuer(), loaded.product.mcp.issuer);
        let verifier = mcp_verifier(&loaded.product.auth, &loaded.product.mcp).await?;
        let claims = server
            .claims("user_123", &config.resource_url, Duration::from_secs(300))?
            .issuer(&config.issuer)
            .claim("client_id", "client_mcp");
        assert_eq!(
            verifier.verify(&server.mint(&claims)?).await?.subject(),
            "user_123"
        );
        for invalid in [
            claims.clone().issuer("https://wrong.example"),
            claims.clone().expires_at(0),
        ] {
            assert!(verifier.verify(&server.mint(&invalid)?).await.is_err());
        }
        let invalid_binding = if provider == AuthProvider::Clerk {
            claims.claim("client_id", "another_resource")
        } else {
            claims.audience("rest-api")
        };
        assert!(
            verifier
                .verify(&server.mint(&invalid_binding)?)
                .await
                .is_err()
        );
        config.introspection_client_id = Some("resource".into());
        config.introspection_client_secret = Some(baukit_config::Secret::new("secret".into()));
        assert_eq!(
            {{ context.app_crate }}_bin::mcp_policy(&auth, &config).is_ok(),
            provider == AuthProvider::Oidc
        );
    }
    Ok(())
}

#[test]
fn startup_rejects_unsupported_introspection_and_unbound_clerk_oauth() -> Result<(), Box<dyn Error>>
{
    for provider in ["oidc", "clerk", "workos"] {
        let file = tempfile::Builder::new().suffix(".toml").tempfile()?;
        std::fs::write(
            file.path(),
            format!(
                "[auth]\nprovider = {provider:?}\nissuer = \"https://identity.example\"\nclient_id = \"client_app\"\n[mcp]\nintrospection_client_id = \"resource\"\nintrospection_client_secret = \"secret\"\n",
            ),
        )?;
        let result = ConfigLoader::new("provider-startup-test", Environment::Local)?
            .local_file(file.path())
            .without_dotenv()
            .load::<ProductConfig>();
        if provider == "oidc" {
            result?;
        } else {
            assert!(
                result
                    .expect_err("managed provider accepted Keycloak introspection")
                    .to_string()
                    .contains("Keycloak introspection")
            );
        }
    }
    let file = tempfile::Builder::new().suffix(".toml").tempfile()?;
    std::fs::write(
        file.path(),
        "[auth]\nprovider = \"clerk\"\nissuer = \"https://identity.example\"\n[mcp]\nenabled = true\nissuer = \"https://identity.example\"\nresource_url = \"https://mcp.example/mcp\"\nallowed_hosts = [\"mcp.example\"]\n",
    )?;
    let error = ConfigLoader::new("provider-startup-test", Environment::Local)?
        .local_file(file.path())
        .without_dotenv()
        .load::<ProductConfig>()
        .expect_err("Clerk MCP started without an OAuth client binding");
    assert!(error.to_string().contains("dedicated OAuth client ID"));
    Ok(())
}

{% endif %}#[path = "support/erasure.rs"]
mod erasure;

#[tokio::test]
#[ignore = "requires Docker PostgreSQL for product erasure fences"]
async fn configured_provider_authenticates_the_product_route_with_the_same_identity()
-> Result<(), Box<dyn Error>> {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use baukit_auth::AuthState;
    use std::sync::Arc;
    use tower::ServiceExt as _;

    use {{ context.app_crate }}_api::ApiState;
    use {{ context.app_crate }}_api::router;
    use {{ context.app_crate }}_bin::InMemoryItemRepository;
    use {{ context.app_crate }}_bin::InMemoryUserRepository;
    use {{ context.app_crate }}_services::ItemService;
    use {{ context.app_crate }}_services::UserService;

    let fixture = erasure::erasure_fixture().await?;
    let server = baukit_test::MockOidcServer::start().await?;
    let issuer = "https://external.example/tenant/";
    let users = UserService::new(Arc::new(InMemoryUserRepository::new()));
    let mut user_id = None;
    for provider in ["oidc", "clerk", "workos"] {
        let file = tempfile::Builder::new().suffix(".toml").tempfile()?;
        let jwks = format!("jwks_uri = {:?}\n", server.jwks_url());
        std::fs::write(
            file.path(),
            format!(
                "[auth]\nprovider = {provider:?}\nissuer = {:?}\naudience = \"api\"\nclient_id = \"client_app\"\nauthorized_parties = [\"https://app.example\"]\n{jwks}",
                issuer,
            ),
        )?;
        let config: BaukitConfig<ProductConfig> =
            ConfigLoader::new("provider-route-test", Environment::Local)?
                .local_file(file.path())
                .without_dotenv()
                .load()?;
        let verifier = auth_verifier(&config.product.auth).await?;
        let app = router(
            ApiState {
                items: ItemService::new(Arc::new(InMemoryItemRepository::new())),
                users: users.clone(),
                auth: AuthState::from_shared(verifier),
                erasure: fixture.erasure.clone(),
            },
            &baukit_config::HttpConfig::default(),
        )?;
        let claims = server
            .claims("user_123", "api", Duration::from_secs(300))?
            .issuer(issuer)
            .claim("azp", "https://app.example")
            .claim("client_id", "client_app");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header(
                        header::AUTHORIZATION,
                        baukit_test::authorization_header(&server.mint(&claims)?)?,
                    )
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
        assert_eq!(body["subject"], "user_123");
        let id = body["id"].as_str().ok_or("missing user ID")?.to_owned();
        uuid::Uuid::parse_str(&id)?;
        if let Some(expected) = &user_id {
            assert_eq!(&id, expected);
        } else {
            user_id = Some(id);
        }
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header(
                        header::AUTHORIZATION,
                        baukit_test::authorization_header(
                            &server.mint(&claims.issuer("https://another.example"))?,
                        )?,
                    )
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    Ok(())
}
