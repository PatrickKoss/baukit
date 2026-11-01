use baukit_mcp::{McpConfig, ScopedTool, tool_schema};
use serde_json::json;

#[test]
fn migration_scope_instructions_match_the_configuration_and_registry() {
    let guide = include_str!("../../../../docs/migrations/mcp-stdio-to-remote.md");
    assert!(!guide.contains("`mcp.scopes_supported`"));
    assert!(guide.contains("Empty required-scopes lists are rejected"));
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
