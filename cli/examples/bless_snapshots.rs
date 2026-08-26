//! Regenerates the golden trees in `tests/snapshots/`. Run with `cargo run --example bless_snapshots`.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use baukit_cli::{AuthProvider, NewOptions, generate_new};
use sha2::{Digest, Sha256};

fn base(parent: &Path) -> NewOptions {
    NewOptions {
        name: "snapshot-app".to_owned(),
        directory: parent.to_path_buf(),
        backend: true,
        worker: false,
        mobile: false,
        web: false,
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
    }
}

fn read_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut tree = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
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

fn render(tree: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut out = String::new();
    for (path, contents) in tree {
        out.push_str(&format!(
            "{:x}  {}\n",
            Sha256::digest(contents),
            path.display()
        ));
    }
    out
}

fn bless(name: &str, mutate: impl FnOnce(&mut NewOptions)) {
    let parent = tempfile::tempdir().expect("tempdir");
    let mut options = base(parent.path());
    mutate(&mut options);
    let root = generate_new(&options).expect("generate");
    let snapshot = render(&read_tree(&root));
    let target = Path::new("tests/snapshots").join(format!("{name}.tree"));
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
    bless("web", |o| {
        o.backend = false;
        o.web = true;
    });
    bless("combined", |o| {
        o.mobile = true;
        o.web = true;
    });
    bless("auth", |o| {
        o.mobile = true;
        o.web = true;
        o.auth = Some(AuthProvider::Oidc);
    });
}
