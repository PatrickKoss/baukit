use super::*;

struct CancellableService {
    started: tokio::sync::mpsc::UnboundedSender<baukit_mcp::CancellationToken>,
    observed: tokio::sync::Notify,
}

impl CancellableService {
    async fn wait(&self, cancellation: baukit_mcp::CancellationToken) {
        self.started
            .send(cancellation.clone())
            .expect("request observer");
        cancellation.cancelled().await;
        self.observed.notify_one();
    }
}

impl ToolService for CancellableService {
    fn tools(&self) -> Vec<ScopedTool> {
        IdentityTool.tools()
    }

    fn call<'a>(
        &'a self,
        _principal: &'a Principal,
        _name: &'a str,
        _arguments: Value,
        cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            self.wait(cancellation).await;
            Ok(json!({"cancelled": true}))
        })
    }
}

impl baukit_mcp::ResourceService for CancellableService {
    fn list(&self) -> Vec<baukit_mcp::ScopedResource> {
        vec![baukit_mcp::ScopedResource {
            resource: baukit_mcp::Resource::new("product://cancel", "cancel"),
            required_scopes: vec!["identity:read".into(), "account:read".into()],
        }]
    }

    fn templates(&self) -> Vec<baukit_mcp::ScopedResourceTemplate> {
        Vec::new()
    }

    fn read<'a>(
        &'a self,
        _principal: &'a Principal,
        uri: &'a str,
        cancellation: baukit_mcp::CancellationToken,
    ) -> baukit_mcp::ResourceFuture<'a> {
        Box::pin(async move {
            self.wait(cancellation).await;
            Ok(vec![baukit_mcp::ResourceContents::text("cancelled", uri)])
        })
    }
}

fn cancellable_message(resource: bool) -> Value {
    if resource {
        json!({"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"product://cancel"}})
    } else {
        call()
    }
}

async fn assert_unrelated_cancellations_are_ignored(
    app: &Router,
    cancel: &Value,
    token: &str,
    foreign: &str,
    cancellation: &baukit_mcp::CancellationToken,
) -> Result<(), Box<dyn Error>> {
    for (caller, id) in [(foreign, json!(2)), (token, json!(999)), (token, json!({}))] {
        let mut notification = cancel.clone();
        notification["params"]["requestId"] = id;
        assert_eq!(
            send(app, Some(caller), notification).await?.0,
            StatusCode::ACCEPTED
        );
        assert!(!cancellation.is_cancelled());
    }
    Ok(())
}

#[tokio::test]
async fn tool_and_resource_calls_observe_client_cancellation_and_disconnect()
-> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let claims = issuer
        .claims("alice", &config.resource_url, Duration::from_secs(300))?
        .claim("scope", "identity:read account:read");
    let token = issuer.mint(&claims.clone().claim("azp", "owner-client"))?;
    let foreign = issuer.mint(&claims.claim("azp", "other-client"))?;
    for resource in [false, true] {
        for disconnect in [false, true] {
            let (started, mut requests) = tokio::sync::mpsc::unbounded_channel();
            let service = Arc::new(CancellableService {
                started,
                observed: Default::default(),
            });
            let app = router(
                config.clone(),
                verifier(&config).await?,
                McpServices::new(service.clone()).with_resources(service.clone()),
                Arc::new(InMemoryRateLimitStore::default()),
                Arc::new(JwtOnlyPolicy),
            )
            .await?;
            let message = cancellable_message(resource);
            let worker_app = app.clone();
            let worker_token = token.clone();
            let worker = tokio::spawn(async move {
                send(&worker_app, Some(&worker_token), message)
                    .await
                    .map_err(|error| error.to_string())
            });
            let cancellation = tokio::time::timeout(Duration::from_secs(5), requests.recv())
                .await?
                .expect("started request");
            assert!(!cancellation.is_cancelled());
            let cancel = json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}});
            assert_unrelated_cancellations_are_ignored(
                &app,
                &cancel,
                &token,
                &foreign,
                &cancellation,
            )
            .await?;
            let duplicate = send(&app, Some(&token), cancellable_message(resource)).await?;
            assert_eq!(duplicate.2["error"]["code"], -32600);
            assert!(!cancellation.is_cancelled());
            if disconnect {
                worker.abort();
                assert!(
                    worker
                        .await
                        .expect_err("disconnected request")
                        .is_cancelled()
                );
            } else {
                let (status, headers, body) = send(&app, Some(&token), cancel).await?;
                assert_eq!(status, StatusCode::ACCEPTED);
                assert_eq!(headers[header::CACHE_CONTROL], "no-store");
                assert_eq!(body, Value::Null);
                let result = tokio::time::timeout(Duration::from_secs(5), worker)
                    .await??
                    .map_err(std::io::Error::other)?;
                assert_eq!(result.0, StatusCode::OK);
                assert!(result.2.get("result").is_some());
            }
            tokio::time::timeout(Duration::from_secs(5), service.observed.notified()).await?;
            assert!(cancellation.is_cancelled());
        }
    }
    Ok(())
}
