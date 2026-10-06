use std::{
    error::Error,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use baukit_mcp::{McpConfig, Principal, ScopedTool, ToolFuture, ToolService};
use baukit_ratelimit::InMemoryRateLimitStore;
use reqwest::Client;
use rmcp::{
    ClientServiceExt,
    model::{CallToolRequestParams, ProtocolVersion},
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
        }]
    }
    fn call<'a>(
        &'a self,
        principal: &'a Principal,
        _name: &'a str,
        _arguments: Value,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            Ok(json!({"subject":principal.subject(), "client":principal.client_id()}))
        })
    }
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

#[tokio::test]
#[ignore = "requires Docker Keycloak"]
async fn real_keycloak_audience_expiry_and_protocol() -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let resource = format!("http://{address}/mcp");
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
        .start()
        .await?;
    let base = format!(
        "http://127.0.0.1:{}",
        container.get_host_port_ipv4(8080).await?
    );
    let client = Client::builder().no_proxy().build()?;
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
    let clients: Vec<Value> = [("mcp", &resource, None), ("wrong", &"https://other.example/mcp".to_owned(), None), ("expired", &resource, Some("1"))].into_iter()
        .map(|(id, audience, lifetime)| {
            let mut attributes = json!({});
            if let Some(lifetime) = lifetime { attributes["access.token.lifespan"] = json!(lifetime); }
            json!({"clientId":id,"enabled":true,"publicClient":false,"secret":"test-secret","standardFlowEnabled":false,"serviceAccountsEnabled":true,"defaultClientScopes":["basic"],"optionalClientScopes":["items:read"],"attributes":attributes,
                "protocolMappers":[{"name":"resource audience","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper","config":{"included.custom.audience":audience,"access.token.claim":"true"}}]})
        }).collect();
    client.post(format!("{base}/admin/realms")).bearer_auth(admin["access_token"].as_str().ok_or("admin token")?)
        .json(&json!({"realm":"mcp-test","enabled":true,"sslRequired":"none","clientScopes":[{"name":"items:read","protocol":"openid-connect","attributes":{"include.in.token.scope":"true"}}],"clients":clients}))
        .send().await?.error_for_status()?;
    let issuer = format!("{base}/realms/mcp-test");
    let config = McpConfig {
        enabled: true,
        resource_url: resource.clone(),
        issuer: issuer.clone(),
        allowed_hosts: vec![address.to_string()],
        ..Default::default()
    };
    let app = baukit_mcp::router(
        config,
        Arc::new(Subject),
        Arc::new(InMemoryRateLimitStore::default()),
    )
    .await?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let access = token(&client, &issuer, "mcp").await?;
    for lifecycle in [
        ClientLifecycleMode::Initialize,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    ] {
        let discover = matches!(lifecycle, ClientLifecycleMode::Discover { .. });
        let transport = StreamableHttpClientTransport::with_client(
            client.clone(),
            StreamableHttpClientTransportConfig::with_uri(resource.clone())
                .auth_header(access["access_token"].as_str().ok_or("access token")?),
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
        peer.cancel().await?;
    }
    let wrong = token(&client, &issuer, "wrong").await?;
    rejected(
        &client,
        &resource,
        wrong["access_token"].as_str().ok_or("wrong token")?,
    )
    .await?;
    let expired = token(&client, &issuer, "expired").await?;
    let received = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let expires_in = expired["expires_in"].as_u64().ok_or("expires_in")?;
    assert_eq!(expires_in, 1);
    let deadline = received + Duration::from_secs(expires_in + 1);
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    tokio::time::sleep(deadline.saturating_sub(now)).await;
    rejected(
        &client,
        &resource,
        expired["access_token"].as_str().ok_or("expired token")?,
    )
    .await?;
    server.abort();
    assert!(server.await.expect_err("server cancelled").is_cancelled());
    Ok(())
}
