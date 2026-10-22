use super::*;
use baukit_mcp::{CancellationToken, SuiteLinksTool, tool_schema};
use baukit_suite::{
    domain::{DeliveryHealth, EncryptedPayload, LinkRole, LinkStatus, RewardMode, SuiteLink},
    services::{SuiteLinksRead, SuiteServiceError},
};
use std::sync::Mutex;
use uuid::Uuid;

struct Links {
    links: Vec<SuiteLink>,
    owners: Mutex<Vec<Uuid>>,
    fail: bool,
    leak: bool,
}

#[async_trait::async_trait]
impl SuiteLinksRead for Links {
    async fn list(&self, owner: Uuid) -> Result<Vec<SuiteLink>, SuiteServiceError> {
        self.owners.lock().expect("owners").push(owner);
        if self.fail {
            return Err(SuiteServiceError::Unavailable);
        }
        Ok(self
            .links
            .iter()
            .filter(|link| self.leak || link.user_id == owner)
            .cloned()
            .collect())
    }
}

fn test_link(owner: Uuid, id: Uuid) -> SuiteLink {
    let now = "2026-10-06T09:00:00Z".parse().expect("valid fixture");
    SuiteLink {
        id,
        user_id: owner,
        peer_app: "beta".into(),
        role: LinkRole::Initiator,
        remote_link_id: "0199ba71-6000-7000-8000-000000000003"
            .parse()
            .expect("valid fixture"),
        remote_subject: "sender-subject".into(),
        remote_display_name: None,
        suite_subject: None,
        status: LinkStatus::Active,
        secret: EncryptedPayload {
            ciphertext: vec![1],
            nonce: vec![2],
            key_version: 1,
        },
        sends: vec!["alpha.activity.completed".into()],
        receives: vec!["beta.activity.completed".into()],
        share_xp: true,
        reward_mode: RewardMode::Native,
        delivery_health: DeliveryHealth::Healthy,
        consecutive_failures: 0,
        last_delivery_at: None,
        last_failure_at: None,
        last_failure_code: None,
        last_received_at: None,
        created_at: now,
        updated_at: now,
        revoked_at: None,
    }
}

fn reader(links: Vec<SuiteLink>) -> Arc<Links> {
    Arc::new(Links {
        links,
        owners: Mutex::new(Vec::new()),
        fail: false,
        leak: false,
    })
}

#[tokio::test]
async fn suite_tool_enforces_mcp_scope_and_uses_only_the_verified_owner()
-> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let id = Uuid::from_u128(3);
    let links = reader(vec![
        test_link(owner, id),
        test_link(other, Uuid::from_u128(4)),
    ]);
    let services = McpServices::new(Arc::new(IdentityTool)).with_suite_links(links.clone());
    let schema = baukit_mcp::service_schema(&services)?;
    assert_eq!(
        schema["tools"]["scopes"]["list_suite_links"],
        json!(["links:read"])
    );
    let app = router(
        config.clone(),
        verifier(&config).await?,
        services,
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    let claims = issuer.claims(
        owner.to_string(),
        &config.resource_url,
        Duration::from_secs(300),
    )?;
    let message = json!({"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"list_suite_links","arguments":{}}});
    let denied = issuer.mint(&claims.clone().claim("scope", "items:read"))?;
    let (status, headers, body) = send(&app, Some(&denied), message.clone()).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, json!({"error":"insufficient_scope"}));
    assert!(
        headers[header::WWW_AUTHENTICATE]
            .to_str()?
            .contains("scope=\"links:read\"")
    );
    assert!(links.owners.lock().expect("owners").is_empty());
    let (_, _, listed) = send(&app, Some(&denied), list()).await?;
    assert_eq!(listed["result"]["tools"], json!([]));
    let rest_token = issuer.mint(
        &issuer
            .claims(owner.to_string(), "product-rest", Duration::from_secs(300))?
            .claim("scope", "links:read"),
    )?;
    let (status, _, _) = send(&app, Some(&rest_token), message.clone()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(links.owners.lock().expect("owners").is_empty());
    let product_token =
        issuer.mint(&claims.clone().claim("scope", "identity:read account:read"))?;
    let (status, _, product) = send(&app, Some(&product_token), call()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        product["result"]["structuredContent"]["subject"],
        owner.to_string()
    );
    let token = issuer.mint(&claims.claim("scope", "links:read"))?;
    let (status, _, body) = send(&app, Some(&token), message).await?;
    assert_eq!(status, StatusCode::OK);
    let expected = json!({"links":[{
        "id":id,"peerApp":"beta","sends":["alpha.activity.completed"],"receives":["beta.activity.completed"],
        "status":"active","createdAt":"2026-10-06T09:00:00Z","updatedAt":"2026-10-06T09:00:00Z"
    }],"nextCursor":null});
    assert_eq!(body["result"]["structuredContent"], expected);
    jsonschema::validator_for(
        SuiteLinksTool::definition()
            .output_schema
            .as_ref()
            .ok_or("schema")?,
    )?
    .validate(&expected)
    .map_err(|error| error.to_string())?;
    assert_eq!(*links.owners.lock().expect("owners"), [owner]);
    let other_token = issuer.mint(
        &issuer
            .claims(
                other.to_string(),
                &config.resource_url,
                Duration::from_secs(300),
            )?
            .claim("scope", "links:read"),
    )?;
    let (_, _, body) = send(&app, Some(&other_token), json!({"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"list_suite_links","arguments":{}}})).await?;
    assert_eq!(
        body["result"]["structuredContent"]["links"]
            .as_array()
            .ok_or("links")?
            .len(),
        1
    );
    assert_eq!(
        body["result"]["structuredContent"]["links"][0]["id"],
        Uuid::from_u128(4).to_string()
    );
    Ok(())
}

#[tokio::test]
async fn suite_tool_pages_without_duplicates_and_rejects_account_arguments()
-> Result<(), Box<dyn Error>> {
    let owner = Uuid::from_u128(1);
    let source = reader(
        (1..=101)
            .rev()
            .map(|id| test_link(owner, Uuid::from_u128(id)))
            .collect(),
    );
    let tool = SuiteLinksTool::new(source.clone());
    let principal = Principal::new(owner.to_string());
    let page = tool
        .call(
            &principal,
            "list_suite_links",
            json!({}),
            Default::default(),
        )
        .await?;
    assert_eq!(page["links"].as_array().ok_or("links")?.len(), 100);
    assert_eq!(page["nextCursor"], Uuid::from_u128(100).to_string());
    let page = tool
        .call(
            &principal,
            "list_suite_links",
            json!({"after":page["nextCursor"]}),
            Default::default(),
        )
        .await?;
    assert_eq!(page["links"].as_array().ok_or("links")?.len(), 1);
    assert_eq!(page["links"][0]["id"], Uuid::from_u128(101).to_string());
    assert!(page["nextCursor"].is_null());
    for args in [
        json!({"owner":Uuid::nil()}),
        json!({"after":"invalid"}),
        json!({"after":null}),
        json!([]),
        json!([null]),
    ] {
        assert_eq!(
            tool.call(&principal, "list_suite_links", args, Default::default())
                .await
                .expect_err("arguments")
                .code,
            "invalid_arguments"
        );
    }
    assert_eq!(
        tool.call(
            &Principal::new("unmapped-subject"),
            "list_suite_links",
            json!({}),
            Default::default()
        )
        .await
        .expect_err("identity")
        .code,
        "invalid_identity"
    );
    assert_eq!(
        tool.call(&principal, "other_tool", json!({}), Default::default())
            .await
            .expect_err("tool")
            .code,
        "unknown_tool"
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        tool.call(&principal, "list_suite_links", json!({}), cancelled)
            .await
            .expect_err("cancelled")
            .code,
        "cancelled"
    );
    assert_eq!(source.owners.lock().expect("owners").len(), 2);
    let empty = SuiteLinksTool::new(reader(Vec::new()));
    assert_eq!(
        empty
            .call(
                &principal,
                "list_suite_links",
                json!({}),
                Default::default()
            )
            .await?,
        json!({"links":[],"nextCursor":null})
    );
    let schema = tool_schema(&tool.tools())?;
    assert!(schema.to_string().contains("list_suite_links"));
    Ok(())
}

#[tokio::test]
async fn suite_tool_redacts_service_errors_and_rejects_foreign_owner_rows() {
    let owner = Uuid::from_u128(1);
    let principal = Principal::new(owner.to_string());
    for source in [
        Links {
            links: Vec::new(),
            owners: Mutex::new(Vec::new()),
            fail: true,
            leak: false,
        },
        Links {
            links: vec![test_link(Uuid::from_u128(2), Uuid::from_u128(3))],
            owners: Mutex::new(Vec::new()),
            fail: false,
            leak: true,
        },
    ] {
        let error = SuiteLinksTool::new(Arc::new(source))
            .call(
                &principal,
                "list_suite_links",
                json!({}),
                Default::default(),
            )
            .await
            .expect_err("service");
        assert_eq!(error.code, "service_unavailable");
        assert_eq!(error.message, "Suite links are unavailable");
    }
}
