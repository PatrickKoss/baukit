use std::error::Error;

use {{ context.app_crate }}_mcp::ItemTools;

fn main() -> Result<(), Box<dyn Error>> {
    println!("{}", serde_json::to_string_pretty(&ItemTools::schema()?)?);
    Ok(())
}
