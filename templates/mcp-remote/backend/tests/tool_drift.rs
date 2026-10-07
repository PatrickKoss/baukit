use baukit_mcp::tool_schema;
use serde_json::Value;

use {{ context.app_crate }}_mcp::ItemTools;

fn committed_schema() -> Value {
    serde_json::from_str(include_str!("../mcp-tools.json")).expect("committed schema")
}

#[test]
fn registered_tools_match_committed_schema() {
    let actual = tool_schema(&ItemTools::definitions()).expect("tool schema");
    assert_eq!(
        actual,
        committed_schema()["tools"],
        "update mcp-tools.json after reviewing tool schema and scope changes"
    );
}

#[test]
fn registered_resources_and_prompts_match_committed_schema() {
    let actual = ItemTools::schema().expect("capability schema");
    let expected = committed_schema();
    assert_eq!(
        actual["resources"], expected["resources"],
        "update mcp-tools.json after reviewing resource definitions and scopes"
    );
    assert_eq!(
        actual["prompts"], expected["prompts"],
        "update mcp-tools.json after reviewing prompt arguments and scopes"
    );
}
