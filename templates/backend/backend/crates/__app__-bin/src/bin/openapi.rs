use std::{env, error::Error, io::Write as _, path::PathBuf};

use {{ context.app_crate }}_api::openapi_document;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("backend/openapi.json"));
    baukit_openapi::write_schema(&openapi_document(), &path)?;
    writeln!(std::io::stdout().lock(), "wrote {}", path.display())?;
    Ok(())
}
