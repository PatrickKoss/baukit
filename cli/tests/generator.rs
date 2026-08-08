use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use baukit_cli::{NewOptions, doctor, generate_new};
use sha2::{Digest, Sha256};

fn options(parent: &Path, name: &str) -> NewOptions {
    NewOptions {
        name: name.to_owned(),
        directory: parent.to_path_buf(),
        backend: true,
        mobile: false,
        web: false,
        force: false,
        baukit_path: None,
    }
}

fn frontend_options(parent: &Path, name: &str, mobile: bool, web: bool) -> NewOptions {
    NewOptions {
        name: name.to_owned(),
        directory: parent.to_path_buf(),
        backend: false,
        mobile,
        web,
        force: false,
        baukit_path: None,
    }
}

#[test]
fn backend_generation_matches_golden_tree_and_is_deterministic() -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let first = generate_new(&options(first_parent.path(), "snapshot-app"))?;
    let second = generate_new(&options(second_parent.path(), "snapshot-app"))?;

    let first_tree = read_tree(&first)?;
    let second_tree = read_tree(&second)?;
    assert_eq!(
        first_tree, second_tree,
        "same inputs must produce identical bytes"
    );

    let actual = render_hash_snapshot(&first_tree);
    let expected = include_str!("snapshots/backend.tree");
    assert_eq!(actual, expected, "generated backend tree changed");
    Ok(())
}

#[test]
fn mobile_generation_matches_golden_tree_and_is_deterministic() -> anyhow::Result<()> {
    assert_deterministic_snapshot(
        |parent| frontend_options(parent, "snapshot-app", true, false),
        include_str!("snapshots/mobile.tree"),
    )
}

#[test]
fn web_generation_matches_golden_tree_and_is_deterministic() -> anyhow::Result<()> {
    assert_deterministic_snapshot(
        |parent| frontend_options(parent, "snapshot-app", false, true),
        include_str!("snapshots/web.tree"),
    )
}

#[test]
fn combined_generation_matches_golden_tree_and_records_capabilities() -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let mut first_options = options(first_parent.path(), "snapshot-app");
    first_options.mobile = true;
    first_options.web = true;
    let mut second_options = options(second_parent.path(), "snapshot-app");
    second_options.mobile = true;
    second_options.web = true;

    let first = generate_new(&first_options)?;
    let second = generate_new(&second_options)?;
    let first_tree = read_tree(&first)?;
    let second_tree = read_tree(&second)?;
    assert_eq!(
        first_tree, second_tree,
        "combined generation must be stable"
    );
    assert_eq!(
        render_hash_snapshot(&first_tree),
        include_str!("snapshots/combined.tree"),
        "generated combined tree changed"
    );

    let manifest = baukit_cli::read_manifest(&first)?;
    assert!(manifest.capabilities.backend);
    assert!(manifest.capabilities.mobile);
    assert!(manifest.capabilities.web);
    assert!(first.join("backend/Cargo.toml").is_file());
    assert!(first.join("mobile/App.tsx").is_file());
    assert!(first.join("web/src/App.tsx").is_file());
    Ok(())
}

#[test]
fn force_reports_conflicts_without_overwriting() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut options = options(parent.path(), "conflict-app");
    let root = generate_new(&options)?;
    let readme = root.join("README.md");
    fs::write(&readme, "user-owned content\n")?;

    assert!(generate_new(&options).is_err());
    options.force = true;
    let error = generate_new(&options).expect_err("modified file must be a conflict");
    assert!(error.to_string().contains("conflict"));
    assert_eq!(fs::read_to_string(readme)?, "user-owned content\n");
    let report = fs::read_to_string(root.join("baukit-conflicts.txt"))?;
    assert!(report.contains("README.md"));
    Ok(())
}

#[test]
fn at_least_one_capability_is_required() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let empty = frontend_options(parent.path(), "empty-app", false, false);
    let error = generate_new(&empty).expect_err("empty capability selection must fail");
    assert!(error.to_string().contains("at least one capability"));
    assert!(!parent.path().join("empty-app").exists());
    Ok(())
}

#[test]
fn doctor_validates_a_local_generated_product() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut local = options(parent.path(), "doctor-app");
    local.mobile = true;
    local.web = true;
    local.baukit_path = Some(baukit_path);
    let root = generate_new(&local)?;
    let results = doctor(&root)?;
    assert!(results.iter().any(|result| result.contains("schema")));
    assert!(
        results
            .iter()
            .any(|result| result.contains("Cargo workspace"))
    );
    assert!(results.iter().any(|result| result.contains("mobile")));
    assert!(results.iter().any(|result| result.contains("web")));
    Ok(())
}

fn assert_deterministic_snapshot(
    make_options: impl Fn(&Path) -> NewOptions,
    expected: &str,
) -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let first = generate_new(&make_options(first_parent.path()))?;
    let second = generate_new(&make_options(second_parent.path()))?;
    let first_tree = read_tree(&first)?;
    let second_tree = read_tree(&second)?;
    assert_eq!(
        first_tree, second_tree,
        "same inputs must produce identical bytes"
    );
    assert_eq!(render_hash_snapshot(&first_tree), expected);
    Ok(())
}

fn read_tree(root: &Path) -> anyhow::Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> anyhow::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, files)?;
            } else {
                files.insert(path.strip_prefix(root)?.to_path_buf(), fs::read(path)?);
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn render_hash_snapshot(tree: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut snapshot = String::new();
    for (path, contents) in tree {
        let digest = Sha256::digest(contents);
        snapshot.push_str(&format!("{digest:x}  {}\n", path.display()));
    }
    snapshot
}
