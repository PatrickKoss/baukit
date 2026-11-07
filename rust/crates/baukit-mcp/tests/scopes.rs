#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "integration test code: a failed expectation is the test failure, and scenario tests read top to bottom"
)]
use baukit_mcp::{McpConfig, ScopedTool, tool_schema};
use serde_json::json;

#[test]
fn scope_configuration_matches_the_registry() {
    assert!(serde_json::from_value::<McpConfig>(json!({"scopes_supported": []})).is_err());
    let mut tool = ScopedTool {
        name: "read".into(),
        description: "Read product data".into(),
        input_schema: json!({"type": "object"}),
        output_schema: None,
        required_scopes: Vec::new(),
        read_only: true,
        annotations: Default::default(),
    };
    assert!(tool_schema(&[tool.clone()]).is_err());
    tool.required_scopes = vec!["items:read".into()];
    assert!(tool_schema(&[tool]).is_ok());
    let metadata = McpConfig::default().metadata(vec!["items:read".into()]);
    assert_eq!(metadata.scopes_supported, ["items:read"]);
}
