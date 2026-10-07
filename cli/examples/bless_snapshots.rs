//! Regenerates the golden trees in `tests/snapshots/`. Run with `cargo run --example bless_snapshots`.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use baukit_cli::{AuthProvider, NewOptions, QualityProfile, generate_new};
use sha2::{Digest, Sha256};

fn base(parent: &Path) -> NewOptions {
    NewOptions {
        name: "snapshot-app".to_owned(),
        directory: parent.to_path_buf(),
        backend: true,
        worker: false,
        mobile: false,
        web: false,
        pwa: false,
        mcp: false,
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
        quality: QualityProfile::Standard,
    }
}

fn read_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut tree = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if is_python_cache_artifact(&path) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).expect("strip").to_path_buf();
                tree.insert(relative, fs::read(&path).expect("read"));
            }
        }
    }
    tree
}

fn is_python_cache_artifact(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__pycache__")
        || path.extension().is_some_and(|extension| extension == "pyc")
}

fn render(tree: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut out = String::new();
    for (path, contents) in tree {
        out.push_str(&format!("{}  {}\n", sha256_hex(contents), path.display()));
    }
    out
}

fn sha256_hex(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bless(name: &str, mutate: impl FnOnce(&mut NewOptions)) {
    let parent = tempfile::tempdir().expect("tempdir");
    let mut options = base(parent.path());
    mutate(&mut options);
    let root = generate_new(&options).expect("generate");
    let snapshot = render(&read_tree(&root));
    let target = snapshot_path(name);
    fs::write(&target, snapshot).expect("write");
    println!("blessed {}", target.display());
}

fn main() {
    bless("backend", |_| {});
    bless("worker", |o| o.worker = true);
    bless("mobile", |o| {
        o.backend = false;
        o.mobile = true;
    });
    bless("mobile-pwa", |o| {
        o.backend = false;
        o.mobile = true;
        o.pwa = true;
    });
    bless("web", |o| {
        o.backend = false;
        o.web = true;
    });
    bless("combined", |o| {
        o.mobile = true;
        o.web = true;
    });
    bless("mcp-remote", |o| {
        o.mcp = true;
        o.auth = Some(AuthProvider::Oidc);
    });
    bless("strict", |o| {
        o.mobile = true;
        o.web = true;
        o.quality = QualityProfile::Strict;
    });
    for (name, provider) in [
        ("clerk", AuthProvider::Clerk),
        ("workos", AuthProvider::Workos),
    ] {
        bless(name, |o| {
            o.mobile = true;
            o.web = true;
            o.mcp = true;
            o.auth = Some(provider);
        });
    }
    bless("auth", |o| {
        o.mobile = true;
        o.web = true;
        o.auth = Some(AuthProvider::Oidc);
    });
}

fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.tree"))
}

#[test]
fn snapshot_path_uses_the_cli_manifest_directory() {
    let target = snapshot_path("backend");
    assert!(target.is_absolute());
    assert_eq!(
        target.parent(),
        Some(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/snapshots")
                .as_path()
        )
    );
}
