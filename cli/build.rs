use std::{env, fs, path::Path};

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=../templates");
    println!("cargo:rerun-if-changed=../typescript/pnpm-lock.yaml");
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../typescript/packages");
    let mut manifests = fs::read_dir(&packages)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("package.json"))
                .filter(|path| path.is_file())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    manifests.sort();
    let mut source = String::from("&[\n");
    for manifest in manifests {
        println!("cargo:rerun-if-changed={}", manifest.display());
        let path = manifest.canonicalize().unwrap_or(manifest);
        source.push_str(&format!(
            "    include_str!({:?}),\n",
            path.display().to_string()
        ));
    }
    source.push(']');
    let out =
        Path::new(&env::var("OUT_DIR").unwrap_or_default()).join("baukit_package_manifests.rs");
    fs::write(out, source)
}
