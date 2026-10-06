use std::error::Error;

use baukit_mcp::tool_schema;
use {{ context.app_crate }}_mcp::ItemTools;

fn main() -> Result<(), Box<dyn Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&tool_schema(&ItemTools::definitions())?)?
    );
    Ok(())
}
