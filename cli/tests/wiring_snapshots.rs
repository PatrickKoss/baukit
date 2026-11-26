use std::fs;

use baukit_cli::generate_new;

#[path = "support/snapshot_flavors.rs"]
mod snapshot_flavors;
#[path = "support/snapshot_wiring.rs"]
mod snapshot_wiring;

use snapshot_flavors::snapshot_flavors;
use snapshot_wiring::{render_wiring_snapshot, wiring_snapshot_path};

#[test]
fn rendered_api_rate_limit_and_auth_wiring_match_the_snapshots() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut changed = Vec::new();
    for (name, options) in snapshot_flavors(parent.path()) {
        let root = generate_new(&options)?;
        let actual = render_wiring_snapshot(&root)?;
        let path = wiring_snapshot_path(name);
        let expected = fs::read_to_string(&path).ok();
        if actual != expected {
            changed.push(name);
        }
        fs::remove_dir_all(&root)?;
    }
    assert!(
        changed.is_empty(),
        "wiring changed for {changed:?}; review the diff after `cargo run --example bless_snapshots`"
    );
    Ok(())
}
