use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const WIRING_FILES: [&str; 6] = [
    "src/bin/api.rs",
    "src/compose.rs",
    "src/rate_limits.rs",
    "src/identity.rs",
    "src/browser_session.rs",
    "src/lib.rs",
];

/// The rendered API, rate-limit and auth wiring of a generated product, as one text with a
/// header per file. `None` when the product has no backend.
pub fn render_wiring_snapshot(root: &Path) -> Result<Option<String>> {
    let Some(bin_crate) = bin_crate(root)? else {
        return Ok(None);
    };
    let mut snapshot = String::new();
    for file in WIRING_FILES {
        let path = bin_crate.join(file);
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        snapshot.push_str(&format!("===== {file} =====\n{text}"));
        if !text.ends_with('\n') {
            snapshot.push('\n');
        }
    }
    Ok(Some(snapshot))
}

pub fn wiring_snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/wiring")
        .join(format!("{name}.rs.txt"))
}

fn bin_crate(root: &Path) -> Result<Option<PathBuf>> {
    let crates = root.join("backend/crates");
    if !crates.is_dir() {
        return Ok(None);
    }
    for entry in fs::read_dir(&crates)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-bin"))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}
