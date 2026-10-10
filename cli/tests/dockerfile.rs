use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};
use baukit_cli::{NewOptions, QualityProfile, generate_new, read_manifest};

fn product(parent: &Path) -> Result<PathBuf> {
    generate_new(&NewOptions {
        name: "image-product".to_owned(),
        directory: parent.to_owned(),
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
    })
}

fn cli(root: &Path, args: &[&str]) -> Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_baukit"))
        .current_dir(root)
        .args(args)
        .output()?)
}

#[test]
fn generate_check_and_doctor_cover_clean_stale_and_missing_files() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = product(parent.path())?;
    let path = root.join("backend/Dockerfile");
    let initial = fs::read(&path)?;
    for args in [vec!["generate", "dockerfile", "--check"], vec!["doctor"]] {
        let output = cli(&root, &args)?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(&path, b"# stale\n")?;
    let output = cli(&root, &["generate", "dockerfile", "--check"])?;
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("near line 1"));
    assert!(error.contains("git diff -- backend/Dockerfile"));
    assert_eq!(fs::read(&path)?, b"# stale\n");
    let output = cli(&root, &["doctor"])?;
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("backend Dockerfile `backend/Dockerfile` differs"));
    assert!(error.contains("run `baukit generate dockerfile`"));
    ensure!(cli(&root, &["generate", "dockerfile"])?.status.success());
    assert_eq!(fs::read(&path)?, initial);
    fs::remove_file(&path)?;
    let output = cli(&root, &["doctor"])?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("missing expected backend file `backend/Dockerfile`")
    );
    assert!(
        !cli(&root, &["generate", "dockerfile", "--check"])?
            .status
            .success()
    );
    ensure!(cli(&root, &["generate", "dockerfile"])?.status.success());
    ensure!(cli(&root, &["doctor"])?.status.success());
    Ok(())
}

#[test]
fn generation_respects_output_override_and_manifest_options() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = product(parent.path())?;
    let mut manifest = read_manifest(&root)?;
    manifest.doctor.backend_dockerfile = Some("build/backend.Dockerfile".to_owned());
    manifest.backend.image.cargo_build_jobs = Some(6);
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    ensure!(cli(&root, &["generate", "dockerfile"])?.status.success());
    assert!(
        fs::read_to_string(root.join("build/backend.Dockerfile"))?
            .contains("ARG CARGO_BUILD_JOBS=6")
    );
    assert!(
        !fs::read_to_string(root.join("backend/Dockerfile"))?.contains("ARG CARGO_BUILD_JOBS=6")
    );
    fs::remove_file(root.join("backend/Dockerfile"))?;
    ensure!(cli(&root, &["doctor"])?.status.success());
    ensure!(
        cli(&root, &["generate", "dockerfile", "--check"])?
            .status
            .success()
    );
    Ok(())
}

#[test]
fn generation_defaults_to_the_declared_backend_manifest_directory() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = product(parent.path())?;
    fs::rename(root.join("backend"), root.join("server"))?;
    fs::remove_file(root.join("server/Dockerfile"))?;
    let mut manifest = read_manifest(&root)?;
    manifest.doctor.backend_manifest = Some("server/Cargo.toml".to_owned());
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    ensure!(cli(&root, &["generate", "dockerfile"])?.status.success());
    assert!(root.join("server/Dockerfile").is_file());
    assert!(!root.join("backend").exists());
    ensure!(
        cli(&root, &["generate", "dockerfile", "--check"])?
            .status
            .success()
    );
    Ok(())
}

#[test]
fn manifest_validation_precedes_generation_and_doctor() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = product(parent.path())?;
    let original = fs::read(root.join("backend/Dockerfile"))?;
    let mut source = fs::read_to_string(root.join("baukit.toml"))?;
    source.push_str(
        "\n[backend.image]\nbuild_inputs = [{ source = '../outside', destination = '/inputs' }]\n",
    );
    fs::write(root.join("baukit.toml"), source)?;
    for args in [vec!["generate", "dockerfile"], vec!["doctor"]] {
        let output = cli(&root, &args)?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("without `..`"));
    }
    assert_eq!(fs::read(root.join("backend/Dockerfile"))?, original);
    Ok(())
}

struct DockerResources {
    builder: Option<String>,
    images: Vec<String>,
    containers: Vec<String>,
}

impl DockerResources {
    fn cleanup(&mut self) -> Result<()> {
        while let Some(container) = self.containers.last() {
            docker(&["rm", "-f", container])?;
            self.containers.pop();
        }
        while let Some(image) = self.images.last() {
            docker(&["image", "rm", "-f", image])?;
            self.images.pop();
        }
        if let Some(builder) = &self.builder {
            docker(&["buildx", "rm", "--force", builder])?;
            self.builder = None;
        }
        Ok(())
    }
}

impl Drop for DockerResources {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("Docker fixture cleanup failed: {error:#}");
        }
    }
}

fn docker(args: &[&str]) -> Result<Output> {
    let output = Command::new("docker").args(args).output()?;
    ensure!(
        output.status.success(),
        "docker {args:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn copied_file(container: &str, source: &str, destination: &Path) -> Result<Vec<u8>> {
    docker(&[
        "cp",
        &format!("{container}:{source}"),
        destination.to_str().context("UTF-8 path")?,
    ])?;
    Ok(fs::read(destination)?)
}

#[test]
#[ignore = "requires Docker"]
fn buildkit_builds_every_binary_with_stage_specific_files() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = product(parent.path())?;
    configure_build_fixture(&root)?;
    let suffix = root
        .parent()
        .context("fixture parent")?
        .file_name()
        .context("fixture name")?
        .to_string_lossy()
        .replace('.', "");
    let builder = format!("baukit-image-test-{suffix}").to_lowercase();
    let mut resources = DockerResources {
        builder: None,
        images: Vec::new(),
        containers: Vec::new(),
    };
    docker(&[
        "buildx",
        "create",
        "--name",
        &builder,
        "--driver",
        "docker-container",
    ])?;
    resources.builder = Some(builder.clone());
    let result = (|| -> Result<()> {
        for binary in ["api", "migrate", "worker", "seed"] {
            let tag = format!("{builder}:{binary}");
            docker(&[
                "buildx",
                "build",
                "--builder",
                &builder,
                "--load",
                "--target",
                binary,
                "--tag",
                &tag,
                "--file",
                root.join("backend/Dockerfile")
                    .to_str()
                    .context("Dockerfile path")?,
                root.to_str().context("context path")?,
            ])?;
            resources.images.push(tag.clone());
            let container = String::from_utf8(docker(&["create", &tag])?.stdout)?
                .trim()
                .to_owned();
            resources.containers.push(container.clone());
            verify_runtime_stage(&container, binary, parent.path())?;
        }
        Ok(())
    })();
    let cleanup = resources.cleanup();
    result?;
    cleanup?;
    Ok(())
}

fn configure_build_fixture(root: &Path) -> Result<()> {
    let mut manifest = read_manifest(root)?;
    manifest.backend.image = toml::from_str(
        r#"
binaries = ["api", "migrate", "worker", "seed"]
cargo_build_jobs = 6
backend_context = "backend"
build_inputs = [{source = "inputs/build.txt", destination = "/inputs/build.txt"}]
pre_build = [{command = ["sh", "-c", "mkdir -p /generated && cp /inputs/build.txt /generated/output.txt"], outputs = ["/generated"]}]
runtime_files = [{stage = "api", source = "/generated/output.txt", destination = "/app/prebuilt.txt"}, {stage = "worker", source = "assets/worker.txt", destination = "/app/worker.txt"}]
runtime_binaries = [{stage = "migrate", binary = "seed"}]
writable_directories = [{stage = "api", path = "/app/var/artifacts"}, {stage = "worker", path = "/tmp/reports"}]
"#,
    )?;
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    baukit_cli::generate_dockerfile(root, false)?;
    fs::create_dir(root.join("inputs"))?;
    fs::write(root.join("inputs/build.txt"), "compiled input\n")?;
    fs::create_dir(root.join("backend/assets"))?;
    fs::write(root.join("backend/assets/worker.txt"), "worker input\n")?;
    // A dependency-free fixture keeps this test focused on Dockerfile behavior.
    fs::write(
        root.join("backend/Cargo.toml"),
        "[workspace]\nmembers = [\"crates/image-product-bin\"]\nresolver = \"3\"\n",
    )?;
    let crate_root = root.join("backend/crates/image-product-bin");
    fs::write(
        crate_root.join("Cargo.toml"),
        "[package]\nname = \"image-product-bin\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fs::remove_dir_all(crate_root.join("src"))?;
    fs::create_dir_all(crate_root.join("src/bin"))?;
    for binary in ["api", "migrate", "worker", "seed"] {
        fs::write(
            crate_root.join(format!("src/bin/{binary}.rs")),
            "fn main() { print!(\"{}\", include_str!(\"/generated/output.txt\")); }\n",
        )?;
    }
    fs::write(
        root.join("backend/Cargo.lock"),
        "version = 4\n[[package]]\nname = \"image-product-bin\"\nversion = \"0.1.0\"\n",
    )?;
    Ok(())
}

fn verify_runtime_stage(container: &str, binary: &str, output: &Path) -> Result<()> {
    let executable = copied_file(
        container,
        &format!("/app/{binary}"),
        &output.join(format!("{binary}-executable")),
    )?;
    assert!(executable.starts_with(b"\x7fELF"));
    if binary == "api" {
        assert_eq!(
            copied_file(container, "/app/prebuilt.txt", &output.join("prebuilt.txt"))?,
            b"compiled input\n"
        );
    } else {
        assert!(
            !Command::new("docker")
                .args(["cp", &format!("{container}:/app/prebuilt.txt"), "-"])
                .output()?
                .status
                .success()
        );
    }
    if binary == "migrate" {
        assert!(
            copied_file(container, "/app/seed", &output.join("companion-seed"))?
                .starts_with(b"\x7fELF")
        );
        assert!(
            String::from_utf8(copied_file(
                container,
                "/workspace/crates/image-product-bin/Cargo.toml",
                &output.join("runtime-manifest.toml")
            )?)?
            .contains("image-product-bin")
        );
        docker(&[
            "cp",
            &format!("{container}:/workspace/migrations"),
            output
                .join("runtime-migrations")
                .to_str()
                .context("migrations path")?,
        ])?;
        assert!(
            output
                .join("runtime-migrations")
                .read_dir()?
                .next()
                .is_some()
        );
    }
    if binary == "worker" {
        assert_eq!(
            copied_file(container, "/app/worker.txt", &output.join("worker.txt"))?,
            b"worker input\n"
        );
    }
    if let Some(directory) = match binary {
        "api" => Some("/app/var/artifacts"),
        "worker" => Some("/tmp/reports"),
        _ => None,
    } {
        let archive = docker(&["cp", &format!("{container}:{directory}"), "-"])?;
        let archive_path = output.join(format!("{binary}-directory.tar"));
        fs::write(&archive_path, archive.stdout)?;
        let listing = Command::new("tar")
            .args(["--numeric-owner", "-tvf"])
            .arg(archive_path)
            .output()?;
        ensure!(listing.status.success());
        assert!(String::from_utf8_lossy(&listing.stdout).contains("65532/65532"));
        assert!(String::from_utf8_lossy(&listing.stdout).starts_with("drwxr-xr-x"));
    }
    Ok(())
}
