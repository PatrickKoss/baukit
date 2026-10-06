use baukit_mcp::tool_schema;
use {{ context.app_crate }}_mcp::ItemTools;
use serde_json::Value;

#[test]
fn registered_tools_match_committed_schema() {
    let actual = tool_schema(&ItemTools::definitions()).expect("tool schema");
    let expected: Value =
        serde_json::from_str(include_str!("../mcp-tools.json")).expect("committed schema");
    assert_eq!(
        actual, expected,
        "update mcp-tools.json after reviewing tool schema and scope changes"
    );
}
