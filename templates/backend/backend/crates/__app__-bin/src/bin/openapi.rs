use std::{env, error::Error, path::PathBuf};

use {{ context.app_crate }}_api::openapi_document;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("backend/openapi.json"));
    baukit_openapi::write_schema(&openapi_document(), &path)?;
    println!("wrote {}", path.display());
    Ok(())
}
