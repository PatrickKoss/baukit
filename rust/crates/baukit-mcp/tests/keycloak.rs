#![cfg(test)]
#[path = "support/resources.rs"]
mod resources;

use std::{
    error::Error,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use baukit_mcp::{
    KeycloakIntrospectionConfig, KeycloakIntrospectionPolicy, McpConfig, McpServices, Principal,
    ScopedTool, ToolFuture, ToolService,
};
use baukit_ratelimit::InMemoryRateLimitStore;
use reqwest::Client;
use rmcp::{
    ClientServiceExt,
    model::{CallToolRequestParams, ProtocolVersion, ReadResourceRequestParams},
    service::ClientLifecycleMode,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use testcontainers::{
    GenericBuildableImage, ImageExt,
    core::{IntoContainerPort, WaitFor, logs::LogFrame},
    runners::{AsyncBuilder, AsyncRunner},
};
use tokio::net::TcpListener;

struct Subject;
impl ToolService for Subject {
    fn tools(&self) -> Vec<ScopedTool> {
        vec![ScopedTool {
            name: "subject".into(),
            description: "Read verified subject".into(),
            input_schema: json!({"type":"object","properties":{}}),
            output_schema: None,
            required_scopes: vec!["items:read".into()],
            read_only: true,
            annotations: Default::default(),
        }]
    }
    fn call<'a>(
        &'a self,
        principal: &'a Principal,
        _name: &'a str,
        _arguments: Value,
        _cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            Ok(json!({"subject":principal.subject(), "client":principal.client_id()}))
        })
    }
}

fn audience_mappers(resource: &str) -> Value {
    json!([
        {"name":"resource audience","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper","config":{"included.custom.audience":resource,"access.token.claim":"true"}},
        {"name":"backend audience","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper","config":{"included.client.audience":"mcp-resource","access.token.claim":"true"}}
    ])
}

async fn token(client: &Client, issuer: &str, id: &str) -> Result<Value, Box<dyn Error>> {
    Ok(client
        .post(format!("{issuer}/protocol/openid-connect/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", id),
            ("client_secret", "test-secret"),
            ("scope", "items:read"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn rejected(client: &Client, resource: &str, access: &str) -> Result<(), Box<dyn Error>> {
    let response = client
        .post(resource)
        .bearer_auth(access)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        response.headers()["www-authenticate"]
            .to_str()?
            .contains("resource_metadata=")
    );
    assert_eq!(
        response.json::<Value>().await?,
        json!({"error":"invalid_token"})
    );
    Ok(())
}

type KeycloakContainer = testcontainers::ContainerAsync<testcontainers::GenericImage>;
type ServerHandle = tokio::task::JoinHandle<std::io::Result<()>>;

const POLICY_CACHE_TTL: Duration = Duration::from_secs(1);

async fn start_keycloak() -> Result<(KeycloakContainer, String), Box<dyn Error>> {
    let image = GenericBuildableImage::new("baukit-mcp-keycloak-test", "26.8.0")
        .with_dockerfile_string(include_str!("keycloak.Dockerfile"))
        .build_image()
        .await?;
    let container = image
        .with_exposed_port(8080.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Listening on:"))
        .with_env_var("KC_BOOTSTRAP_ADMIN_USERNAME", "admin")
        .with_env_var("KC_BOOTSTRAP_ADMIN_PASSWORD", "test-admin-password")
        .with_env_var("JAVA_OPTS_KC_HEAP", "-Xms128m -Xmx512m")
        .with_env_var("JAVA_OPTS_APPEND", "-XX:ActiveProcessorCount=2")
        .with_log_consumer(|frame: &LogFrame| {
            eprint!("{}", String::from_utf8_lossy(frame.bytes()));
        })
        .with_cmd([
            "start",
            "--optimized",
            "--http-enabled=true",
            "--hostname-strict=false",
        ])
        .with_labels(baukit_test::container_labels())
        .start()
        .await?;
    let base = format!(
        "http://127.0.0.1:{}",
        container.get_host_port_ipv4(8080).await?
    );
    Ok((container, base))
}

async fn admin_token(client: &Client, base: &str) -> Result<String, Box<dyn Error>> {
    let admin: Value = client
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
    Ok(admin["access_token"]
        .as_str()
        .ok_or("admin token")?
        .to_owned())
}

fn realm_clients(resource: &str) -> Vec<Value> {
    let mut clients: Vec<Value> = [("mcp", &resource.to_owned(), None), ("wrong", &"https://other.example/mcp".to_owned(), None), ("expired", &resource.to_owned(), Some("1"))].into_iter()
        .map(|(id, audience, lifetime)| {
            let mut attributes = json!({});
            if let Some(lifetime) = lifetime { attributes["access.token.lifespan"] = json!(lifetime); }
            json!({"clientId":id,"enabled":true,"publicClient":false,"secret":"test-secret","standardFlowEnabled":false,"serviceAccountsEnabled":true,"defaultClientScopes":["basic"],"optionalClientScopes":["items:read"],"attributes":attributes,
                "protocolMappers":audience_mappers(audience)})
        }).collect();
    clients.push(json!({"clientId":"session","enabled":true,"publicClient":false,"secret":"test-secret","standardFlowEnabled":false,"directAccessGrantsEnabled":true,"defaultClientScopes":["basic"],"optionalClientScopes":["items:read"],"protocolMappers":audience_mappers(resource)}));
    clients.push(json!({"clientId":"mcp-resource","enabled":true,"publicClient":false,"secret":"test-secret","standardFlowEnabled":false,"directAccessGrantsEnabled":false,"serviceAccountsEnabled":false}));
    clients
}

async fn create_realm(client: &Client, base: &str, resource: &str) -> Result<(), Box<dyn Error>> {
    let admin = admin_token(client, base).await?;
    client.post(format!("{base}/admin/realms")).bearer_auth(admin)
        .json(&json!({"realm":"mcp-test","enabled":true,"sslRequired":"none","clientScopes":[{"name":"basic","protocol":"openid-connect","attributes":{"include.in.token.scope":"false"},"protocolMappers":[{"name":"sub","protocol":"openid-connect","protocolMapper":"oidc-sub-mapper","config":{"introspection.token.claim":"true","access.token.claim":"true"}}]},{"name":"items:read","protocol":"openid-connect","attributes":{"include.in.token.scope":"true"}}],"clients":realm_clients(resource),"users":[{"username":"mcp-user","firstName":"MCP","lastName":"Test","email":"mcp-user@example.test","enabled":true,"emailVerified":true,"credentials":[{"type":"password","value":"test-user-password","temporary":false}]}]}))
        .send().await?.error_for_status()?;
    Ok(())
}

async fn start_mcp_server(
    listener: TcpListener,
    issuer: &str,
    resource: &str,
) -> Result<ServerHandle, Box<dyn Error>> {
    let address = listener.local_addr()?;
    let config = McpConfig {
        enabled: true,
        resource_url: resource.to_owned(),
        issuer: issuer.to_owned(),
        allowed_hosts: vec![address.to_string()],
        ..Default::default()
    };
    let mut policy_config = KeycloakIntrospectionConfig::new(
        issuer,
        "mcp-resource",
        baukit_config::Secret::new("test-secret".to_owned()),
    );
    policy_config.cache_ttl = POLICY_CACHE_TTL;
    let policy = Arc::new(KeycloakIntrospectionPolicy::new(policy_config)?);
    let app = baukit_mcp::router(
        config,
        Arc::new(
            baukit_auth::OidcVerifier::discover(
                baukit_auth::OidcConfig::new(issuer, resource)?
                    .with_clock_skew(Duration::ZERO)
                    .with_principal_claims(
                        baukit_auth::PrincipalClaimMapping::new().client_id_claim("azp"),
                    ),
            )
            .await?,
        ),
        McpServices::new(Arc::new(Subject)).with_resources(Arc::new(resources::Catalog::default())),
        Arc::new(InMemoryRateLimitStore::default()),
        policy,
    )
    .await?;
    Ok(tokio::spawn(
        async move { axum::serve(listener, app).await },
    ))
}

async fn introspect(client: &Client, issuer: &str, access: &str) -> Result<Value, Box<dyn Error>> {
    Ok(client
        .post(format!("{issuer}/protocol/openid-connect/token/introspect"))
        .basic_auth("mcp-resource", Some("test-secret"))
        .form(&[("token", access), ("token_type_hint", "access_token")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn assert_introspection_matches_verified_identity(
    client: &Client,
    issuer: &str,
    verifier: &baukit_auth::OidcVerifier,
    access: &str,
) -> Result<(), Box<dyn Error>> {
    let introspected = introspect(client, issuer, access).await?;
    assert_eq!(introspected["active"], true);
    let identity = verifier.verify(access).await?;
    assert_eq!(introspected["sub"], identity.subject());
    assert_eq!(
        introspected["client_id"],
        identity.client_id().ok_or("verified client")?
    );
    assert_eq!(
        introspected["iss"],
        identity.issuer().ok_or("verified issuer")?
    );
    Ok(())
}

async fn assert_protocol_lifecycle(
    client: &Client,
    resource: &str,
    access: &str,
    lifecycle: ClientLifecycleMode,
) -> Result<(), Box<dyn Error>> {
    let discover = matches!(lifecycle, ClientLifecycleMode::Discover { .. });
    let transport = StreamableHttpClientTransport::with_client(
        client.clone(),
        StreamableHttpClientTransportConfig::with_uri(resource.to_owned()).auth_header(access),
    );
    let peer = ().serve_with_lifecycle(transport, lifecycle).await?;
    let version = &peer.peer_info().ok_or("peer info")?.protocol_version;
    if discover {
        assert_eq!(version, &ProtocolVersion::V_2026_07_28);
    } else {
        assert!(version.has_initialize());
    }
    let tools = peer.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "subject");
    let result = peer
        .call_tool(CallToolRequestParams::new("subject"))
        .await?;
    assert_eq!(result.is_error, Some(false));
    let output = result.structured_content.ok_or("structured content")?;
    assert_eq!(output["client"], "mcp");
    assert!(
        output["subject"]
            .as_str()
            .is_some_and(|subject| !subject.is_empty())
    );
    let read = peer
        .read_resource(ReadResourceRequestParams::new("product://items/42"))
        .await?;
    let content = serde_json::to_value(&read.contents)?;
    assert_eq!(content[0]["uri"], "product://items/42");
    let data: Value = serde_json::from_str(content[0]["text"].as_str().ok_or("resource text")?)?;
    assert_eq!(data["subject"], output["subject"]);
    peer.cancel().await?;
    Ok(())
}

async fn assert_wrong_audience_and_expired_tokens_rejected(
    client: &Client,
    issuer: &str,
    resource: &str,
) -> Result<(), Box<dyn Error>> {
    let wrong = token(client, issuer, "wrong").await?;
    rejected(
        client,
        resource,
        wrong["access_token"].as_str().ok_or("wrong token")?,
    )
    .await?;
    let expired = token(client, issuer, "expired").await?;
    let received = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let expires_in = expired["expires_in"].as_u64().ok_or("expires_in")?;
    assert_eq!(expires_in, 1);
    let deadline = received + Duration::from_secs(expires_in + 1);
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    tokio::time::sleep(deadline.saturating_sub(now)).await;
    rejected(
        client,
        resource,
        expired["access_token"].as_str().ok_or("expired token")?,
    )
    .await
}

async fn password_session(client: &Client, issuer: &str) -> Result<Value, Box<dyn Error>> {
    Ok(client
        .post(format!("{issuer}/protocol/openid-connect/token"))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "session"),
            ("client_secret", "test-secret"),
            ("username", "mcp-user"),
            ("password", "test-user-password"),
            ("scope", "items:read"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn assert_session_calls_tool_as_user(
    client: &Client,
    resource: &str,
    session_access: &str,
    subject: &str,
) -> Result<(), Box<dyn Error>> {
    let transport = StreamableHttpClientTransport::with_client(
        client.clone(),
        StreamableHttpClientTransportConfig::with_uri(resource.to_owned())
            .auth_header(session_access),
    );
    let peer = ().serve_with_lifecycle(transport, ClientLifecycleMode::Initialize).await?;
    assert_eq!(peer.list_all_tools().await?.len(), 1);
    let output = peer
        .call_tool(CallToolRequestParams::new("subject"))
        .await?
        .structured_content
        .ok_or("session content")?;
    assert_eq!(output["client"], "session");
    assert_eq!(output["subject"], subject);
    peer.cancel().await?;
    Ok(())
}

async fn logout(client: &Client, issuer: &str, session: &Value) -> Result<(), Box<dyn Error>> {
    client
        .post(format!("{issuer}/protocol/openid-connect/logout"))
        .basic_auth("session", Some("test-secret"))
        .form(&[(
            "refresh_token",
            session["refresh_token"].as_str().ok_or("refresh token")?,
        )])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn assert_logout_revokes_session(
    client: &Client,
    issuer: &str,
    resource: &str,
    verifier: &baukit_auth::OidcVerifier,
) -> Result<(), Box<dyn Error>> {
    let session = password_session(client, issuer).await?;
    let session_access = session["access_token"]
        .as_str()
        .ok_or("session access token")?;
    let session_identity = verifier.verify(session_access).await?;
    assert_session_calls_tool_as_user(client, resource, session_access, session_identity.subject())
        .await?;
    logout(client, issuer, &session).await?;
    let introspected = introspect(client, issuer, session_access).await?;
    assert_eq!(introspected["active"], false);
    tokio::time::sleep(POLICY_CACHE_TTL).await;
    assert_eq!(verifier.verify(session_access).await?, session_identity);
    rejected(client, resource, session_access).await?;
    rejected(client, resource, session_access).await
}

#[tokio::test]
#[ignore = "requires Docker Keycloak"]
async fn real_keycloak_audience_expiry_protocol_introspection_and_logout()
-> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let resource = format!("http://{}/mcp", listener.local_addr()?);
    let (_container, base) = start_keycloak().await?;
    let client = baukit_test::test_http_client().no_proxy().build()?;
    create_realm(&client, &base, &resource).await?;
    let issuer = format!("{base}/realms/mcp-test");
    let server = start_mcp_server(listener, &issuer, &resource).await?;
    let access = token(&client, &issuer, "mcp").await?;
    let access = access["access_token"].as_str().ok_or("active token")?;
    let verifier = baukit_auth::OidcVerifier::discover(
        baukit_auth::OidcConfig::new(&issuer, &resource)?.with_principal_claims(
            baukit_auth::PrincipalClaimMapping::new().client_id_claim("azp"),
        ),
    )
    .await?;
    assert_introspection_matches_verified_identity(&client, &issuer, &verifier, access).await?;
    for lifecycle in [
        ClientLifecycleMode::Initialize,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    ] {
        assert_protocol_lifecycle(&client, &resource, access, lifecycle).await?;
    }
    assert_wrong_audience_and_expired_tokens_rejected(&client, &issuer, &resource).await?;
    assert_logout_revokes_session(&client, &issuer, &resource, &verifier).await?;
    server.abort();
    assert!(server.await.expect_err("server cancelled").is_cancelled());
    Ok(())
}
