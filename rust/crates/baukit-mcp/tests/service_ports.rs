mod support;

use std::sync::Arc;

use baukit_mcp::{
    McpServices, Principal, PromptService, ResourceService, ScopedTool, ToolError, ToolFuture,
    ToolService, service_schema,
};
use serde_json::{Value, json};

struct NoTools;
impl ToolService for NoTools {
    fn tools(&self) -> Vec<ScopedTool> {
        Vec::new()
    }
    fn call<'a>(
        &'a self,
        _principal: &'a Principal,
        name: &'a str,
        _arguments: Value,
        _cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            Err(ToolError::new(
                "unknown_tool",
                format!("Unknown tool {name}"),
            ))
        })
    }
}

#[tokio::test]
async fn resource_and_prompt_ports_receive_identity_and_preserve_content_and_safe_errors() {
    let principal = Principal::new("account-42");
    let resources = support::Catalog::default();
    let content = resources
        .read(
            &principal,
            "product://items/42",
            baukit_mcp::CancellationToken::new(),
        )
        .await
        .expect("resource");
    let value = serde_json::to_value(content).expect("content");
    assert_eq!(value[0]["uri"], "product://items/42");
    assert_eq!(value[0]["mimeType"], "application/json");
    let body: Value = serde_json::from_str(value[0]["text"].as_str().expect("text")).expect("data");
    assert_eq!(body["subject"], "account-42");
    assert_eq!(
        *resources.0.lock().expect("reads"),
        [("account-42".into(), "product://items/42".into())]
    );
    let error = resources
        .read(
            &principal,
            "product://items/missing",
            baukit_mcp::CancellationToken::new(),
        )
        .await
        .expect_err("missing resource");
    assert_eq!(error.to_string(), "Item not found");
    let prompts = support::Recommendations::default();
    let result = prompts
        .get(&principal, "next-steps", json!({"language":"de"}))
        .await
        .expect("prompt");
    assert_eq!(
        serde_json::to_value(result.messages).expect("messages"),
        json!([{"role":"user","content":{"type":"text","text":"Recommend practice for account-42 in \"de\""}}])
    );
    assert_eq!(
        *prompts.0.lock().expect("prompts"),
        [("account-42".into(), json!({"language":"de"}))]
    );
    let error = prompts
        .get(&principal, "next-steps", json!({"language":"unavailable"}))
        .await
        .expect_err("unavailable prompt");
    assert_eq!(error.to_string(), "Recommendations unavailable");
}

#[test]
fn optional_services_and_their_definitions_are_exported_only_when_registered() {
    let tools = McpServices::new(Arc::new(NoTools));
    let empty = service_schema(&tools).expect("tools-only schema");
    assert_eq!(empty["resources"]["resources"], json!([]));
    assert_eq!(empty["prompts"]["prompts"], json!([]));
    let services = tools
        .with_resources(Arc::new(support::Catalog::default()))
        .with_prompts(Arc::new(support::Recommendations::default()));
    let schema = service_schema(&services).expect("service schema");
    assert_eq!(
        schema["resources"]["templates"][0]["uriTemplate"],
        "product://items/{id}"
    );
    assert_eq!(
        schema["resources"]["scopes"]["product://private"],
        json!(["items:read", "private:read"])
    );
    assert_eq!(
        schema["prompts"]["prompts"][0]["arguments"][0]["name"],
        "language"
    );
    assert_ne!(schema, empty);
}
