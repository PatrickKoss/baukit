#![cfg(feature = "keycloak")]
use baukit_config::Secret;
use baukit_erasure::{
    IdentityAccountDeleter, IdentityDeletionError, KeycloakAccountDeleter, KeycloakDeletionConfig,
};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::error::Error;
use testcontainers::{
    GenericBuildableImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::{AsyncBuilder, AsyncRunner},
};

async fn admin(client: &Client, base: &str) -> Result<String, Box<dyn Error>> {
    let token: Value = client
        .post(format!(
            "{base}/realms/master/protocol/openid-connect/token"
        ))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", "admin"),
            ("password", "test-admin-password"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(token["access_token"]
        .as_str()
        .ok_or("missing admin token")?
        .into())
}
#[tokio::test]
#[ignore = "requires Docker Keycloak"]
async fn deletes_user_treats_404_as_success_and_requires_manage_users() -> Result<(), Box<dyn Error>>
{
    let image = GenericBuildableImage::new("baukit-erasure-keycloak-test", "26.7.0")
        .with_dockerfile_string("FROM quay.io/keycloak/keycloak:26.7.0\nENV JAVA_OPTS_KC_HEAP=\"-Xms128m -Xmx512m\"\nRUN /opt/keycloak/bin/kc.sh build\n")
        .build_image().await?;
    let container = image
        .with_exposed_port(8080.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Listening on:"))
        .with_env_var("KC_BOOTSTRAP_ADMIN_USERNAME", "admin")
        .with_env_var("KC_BOOTSTRAP_ADMIN_PASSWORD", "test-admin-password")
        .with_cmd([
            "start",
            "--optimized",
            "--http-enabled=true",
            "--hostname-strict=false",
        ])
        .start()
        .await?;
    let base = format!(
        "http://{}:{}",
        container.get_host().await?,
        container.get_host_port_ipv4(8080).await?
    );
    let client = Client::builder().no_proxy().build()?;
    let token = admin(&client, &base).await?;
    client.post(format!("{base}/admin/realms")).bearer_auth(&token)
        .json(&json!({"realm":"erasure", "enabled":true,
            "clients":[{"clientId":"backend", "secret":"test-secret", "publicClient":false, "serviceAccountsEnabled":true}, {"clientId":"unprivileged", "secret":"test-secret", "publicClient":false, "serviceAccountsEnabled":true}],
            "users":[{"username":"service-account-backend", "serviceAccountClientId":"backend", "enabled":true, "clientRoles":{"realm-management":["manage-users"]}}]
        })).send().await?.error_for_status()?;
    let response = client
        .post(format!("{base}/admin/realms/erasure/users"))
        .bearer_auth(&token)
        .json(&json!({"username":"erase-me", "enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    let subject = response
        .headers()
        .get("location")
        .ok_or("missing user location")?
        .to_str()?
        .rsplit('/')
        .next()
        .ok_or("missing user id")?
        .to_owned();
    let config = KeycloakDeletionConfig {
        base_url: base.clone(),
        realm: "erasure".into(),
        client_id: "unprivileged".into(),
        client_secret: Secret::new("test-secret".into()),
        allow_local_http: true,
    };
    assert_eq!(
        KeycloakAccountDeleter::new(config.clone())?
            .delete_account(&subject)
            .await,
        Err(IdentityDeletionError::Permanent)
    );
    assert_eq!(
        client
            .get(format!("{base}/admin/realms/erasure/users/{subject}"))
            .bearer_auth(&token)
            .send()
            .await?
            .status(),
        StatusCode::OK
    );
    let adapter = KeycloakAccountDeleter::new(KeycloakDeletionConfig {
        client_id: "backend".into(),
        ..config
    })?;
    adapter.delete_account(&subject).await?;
    assert_eq!(
        client
            .get(format!("{base}/admin/realms/erasure/users/{subject}"))
            .bearer_auth(&token)
            .send()
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    adapter.delete_account(&subject).await?;
    Ok(())
}
