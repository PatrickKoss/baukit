use super::*;

struct TextErrorTool;

impl ToolService for TextErrorTool {
    fn tools(&self) -> Vec<ScopedTool> {
        vec![ScopedTool {
            name: "draft".into(),
            description: "Read a draft".into(),
            input_schema: json!({"type":"object"}),
            output_schema: Some(
                json!({"type":"object","properties":{"draft":{"type":"string"}},"required":["draft"],"additionalProperties":false}),
            ),
            required_scopes: vec!["draft:read".into()],
            read_only: true,
            annotations: Default::default(),
        }]
    }

    fn call<'a>(
        &'a self,
        _principal: &'a Principal,
        _name: &'a str,
        _arguments: Value,
        _cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async {
            Err(ToolError::new("unavailable", json!({"code":"unavailable","message":"Draft unavailable","requestId":"request-42","retryAfterSeconds":5}).to_string()).text_only())
        })
    }
}

#[tokio::test]
async fn text_only_errors_preserve_a_success_output_schema() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let token = issuer.mint(
        &issuer
            .claims("alice", &config.resource_url, Duration::from_secs(300))?
            .claim("scope", "draft:read"),
    )?;
    let app = router(
        config.clone(),
        verifier(&config).await?,
        McpServices::new(Arc::new(TextErrorTool)),
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    let (status, _, listed) = send(&app, Some(&token), list()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listed["result"]["tools"][0]["outputSchema"],
        TextErrorTool.tools()[0]
            .output_schema
            .clone()
            .expect("schema")
    );
    let (status, _, body) = send(
        &app,
        Some(&token),
        request("tools/call", &json!({"name":"draft","arguments":{}})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert!(body["result"].get("structuredContent").is_none());
    assert_eq!(
        body["result"]["content"].as_array().ok_or("content")?.len(),
        1
    );
    assert_eq!(body["result"]["content"][0]["type"], "text");
    let error: Value = serde_json::from_str(
        body["result"]["content"][0]["text"]
            .as_str()
            .ok_or("text")?,
    )?;
    assert_eq!(
        error,
        json!({"code":"unavailable","message":"Draft unavailable","requestId":"request-42","retryAfterSeconds":5})
    );
    Ok(())
}

async fn assert_capability_request_guards(
    app: &Router,
    token: &str,
    denied: &str,
    method: &str,
    message: &Value,
) -> Result<(), Box<dyn Error>> {
    let (status, headers, body) = send(app, None, message.clone()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
    assert_eq!(body, json!({"error":"invalid_token"}));
    assert!(
        headers[header::WWW_AUTHENTICATE]
            .to_str()?
            .contains("items:read")
    );
    let (status, _, body) = send_with_origin(
        app,
        Some(token),
        message.clone(),
        Some("https://evil.example"),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
    assert_eq!(body, json!({"error":"invalid_host_or_origin"}));
    let (status, headers, body) = send(app, Some(denied), message.clone()).await?;
    if method.ends_with("/list") {
        assert_eq!(status, StatusCode::OK, "{method}");
        let key = match method {
            "resources/list" => "resources",
            "resources/templates/list" => "resourceTemplates",
            _ => "prompts",
        };
        assert_eq!(body["result"][key], json!([]));
        return Ok(());
    }
    assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
    assert_insufficient_scope(status, &headers, &body, "insufficient_scope")
}

fn assert_private_cache(result: &Value) {
    assert_eq!(result["ttlMs"], 0);
    assert_eq!(result["cacheScope"], "private");
}

fn assert_resource_result(method: &str, result: &Value) -> Result<(), Box<dyn Error>> {
    match method {
        "resources/list" => {
            assert_eq!(result["resources"].as_array().ok_or("resources")?.len(), 1);
            assert_eq!(result["resources"][0]["uri"], "product://guide");
            assert_eq!(result["resources"][0]["mimeType"], "application/json");
            assert_private_cache(result);
        }
        "resources/templates/list" => {
            assert_eq!(
                result["resourceTemplates"]
                    .as_array()
                    .ok_or("templates")?
                    .len(),
                1
            );
            assert_eq!(
                result["resourceTemplates"][0]["uriTemplate"],
                "product://items/{id}"
            );
        }
        _ => {
            assert_eq!(result["contents"][0]["uri"], "product://items/42");
            let content: Value =
                serde_json::from_str(result["contents"][0]["text"].as_str().ok_or("text")?)?;
            assert_eq!(content["subject"], "alice");
            assert_private_cache(result);
        }
    }
    Ok(())
}

fn assert_prompt_result(method: &str, result: &Value) -> Result<(), Box<dyn Error>> {
    if method == "prompts/list" {
        assert_eq!(result["prompts"].as_array().ok_or("prompts")?.len(), 2);
        assert_eq!(
            result["prompts"][0]["arguments"][0],
            json!({"name":"language","description":"Output language","required":true})
        );
        assert_private_cache(result);
        return Ok(());
    }
    assert_eq!(result["messages"][0]["role"], "user");
    assert_eq!(
        result["messages"][0]["content"]["text"],
        "Recommend practice for alice in \"de\""
    );
    Ok(())
}

fn assert_conflict_tool_result(result: &Value) -> Result<(), Box<dyn Error>> {
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["content"],
        json!([{"type":"text","text":"Reload revision 7 before saving"}])
    );
    assert_eq!(
        result["structuredContent"]["error"]["details"]["currentRevision"],
        7
    );
    let validator = jsonschema::validator_for(&conflict_schema())?;
    assert!(validator.is_valid(&result["structuredContent"]));
    assert!(!validator.is_valid(&json!({"code":"stale_revision","message":"wrong envelope"})));
    Ok(())
}

fn assert_capability_result(method: &str, result: &Value) -> Result<(), Box<dyn Error>> {
    if method.starts_with("resources/") {
        assert_resource_result(method, result)
    } else if method.starts_with("prompts/") {
        assert_prompt_result(method, result)
    } else {
        assert_conflict_tool_result(result)
    }
}

fn capability_requests() -> Vec<(&'static str, Value)> {
    vec![
        ("resources/list", json!({})),
        ("resources/templates/list", json!({})),
        ("resources/read", json!({"uri":"product://items/42"})),
        ("prompts/list", json!({})),
        (
            "prompts/get",
            json!({"name":"next-steps","arguments":{"language":"de"}}),
        ),
        ("tools/call", json!({"name":"save","arguments":{}})),
    ]
}

async fn assert_resource_read_errors(app: &Router, token: &str) -> Result<(), Box<dyn Error>> {
    for uri in ["product://private", "product://private/42"] {
        let (status, _, _) = send(
            app,
            Some(token),
            request("resources/read", &json!({"uri":uri})),
        )
        .await?;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    for (uri, code, message) in [
        ("product://items/missing", -32602, "Item not found"),
        ("product://items/unavailable", -32603, "Items unavailable"),
        ("product://unknown", -32602, "Unknown resource"),
    ] {
        let (_, _, body) = send(
            app,
            Some(token),
            request("resources/read", &json!({"uri":uri})),
        )
        .await?;
        assert_eq!(body["error"]["code"], code);
        assert_eq!(body["error"]["message"], message);
    }
    Ok(())
}

async fn assert_prompt_get_errors(
    app: &Router,
    token: &str,
    recommendations: &support::Recommendations,
) -> Result<(), Box<dyn Error>> {
    let count = recommendations.0.lock().expect("prompts").len();
    for params in [
        json!({"name":"next-steps"}),
        json!({"name":"next-steps","arguments":{"language":42}}),
        json!({"name":"next-steps","arguments":{"language":"de","extra":"value"}}),
        json!({"name":"unknown"}),
    ] {
        let (_, _, body) = send(app, Some(token), request("prompts/get", &params)).await?;
        assert_eq!(body["error"]["code"], -32602);
    }
    assert_eq!(recommendations.0.lock().expect("prompts").len(), count);
    let (_, _, result) = send(
        app,
        Some(token),
        request("prompts/get", &json!({"name":"recommend-next-steps"})),
    )
    .await?;
    assert_eq!(result["result"]["messages"][0]["role"], "user");
    let (_, _, result) = send(
        app,
        Some(token),
        request(
            "prompts/get",
            &json!({"name":"next-steps","arguments":{"language":"unavailable"}}),
        ),
    )
    .await?;
    assert_eq!(result["error"]["message"], "Recommendations unavailable");
    Ok(())
}

#[tokio::test]
async fn optional_capabilities_use_the_real_router_and_scope_grants() -> Result<(), Box<dyn Error>>
{
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let claims = issuer.claims("alice", &config.resource_url, Duration::from_secs(300))?;
    let token = issuer.mint(
        &claims
            .clone()
            .claim("scope", "items:read learning:read items:write"),
    )?;
    let denied = issuer.mint(&claims.clone().claim("scope", "unrelated:read"))?;
    let catalog = Arc::new(support::Catalog::default());
    let recommendations = Arc::new(support::Recommendations::default());
    let services = McpServices::new(Arc::new(ConflictTool))
        .with_resources(catalog.clone())
        .with_prompts(recommendations.clone());
    let app = router(
        config.clone(),
        verifier(&config).await?,
        services,
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    let (_, _, discovered) =
        send(&app, Some(&token), request("server/discover", &json!({}))).await?;
    assert_eq!(discovered["result"]["capabilities"]["resources"], json!({}));
    assert_eq!(discovered["result"]["capabilities"]["prompts"], json!({}));
    for (method, params) in capability_requests() {
        let message = request(method, &params);
        assert_capability_request_guards(&app, &token, &denied, method, &message).await?;
        let (status, headers, body) = send(&app, Some(&token), message).await?;
        assert_eq!(status, StatusCode::OK, "{method}: {body}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_capability_result(method, &body["result"])?;
    }
    assert_eq!(
        *catalog.0.lock().expect("reads"),
        [("alice".into(), "product://items/42".into())]
    );
    assert_eq!(
        *recommendations.0.lock().expect("prompts"),
        [("alice".into(), json!({"language":"de"}))]
    );
    assert_resource_read_errors(&app, &token).await?;
    assert_prompt_get_errors(&app, &token, &recommendations).await
}
