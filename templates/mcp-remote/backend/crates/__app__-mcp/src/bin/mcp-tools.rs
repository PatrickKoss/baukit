use std::{error::Error, io::Write};

use {{ context.app_crate }}_mcp::ItemTools;

fn main() -> Result<(), Box<dyn Error>> {
    let schema = serde_json::to_string_pretty(&ItemTools::schema()?)?;
    writeln!(std::io::stdout().lock(), "{schema}")?;
    Ok(())
}
