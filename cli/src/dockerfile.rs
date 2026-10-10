use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use minijinja::{Environment, context};
use serde::{Deserialize, Serialize};

use crate::{Manifest, doctor_layout, read_manifest, validate_name};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BackendManifest {
    #[serde(skip_serializing_if = "BackendImage::is_empty")]
    pub image: BackendImage,
}

impl BackendManifest {
    pub fn is_empty(&self) -> bool {
        self.image.is_empty()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BackendImage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binaries: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin_crate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo_build_jobs: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<BuildInput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pre_build: Vec<PreBuildStep>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runtime_files: Vec<RuntimeFile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runtime_binaries: Vec<RuntimeBinary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub writable_directories: Vec<WritableDirectory>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<Download>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub apt_packages: Vec<String>,
}

impl BackendImage {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    fn binaries(&self, worker: bool) -> Vec<String> {
        self.binaries.clone().unwrap_or_else(|| {
            let mut binaries = vec!["api".to_owned(), "migrate".to_owned()];
            if worker {
                binaries.push("worker".to_owned());
            }
            binaries
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildInput {
    pub source: String,
    pub destination: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreBuildStep {
    pub command: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFile {
    pub stage: String,
    pub source: String,
    pub destination: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBinary {
    pub stage: String,
    pub binary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WritableDirectory {
    pub stage: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Download {
    pub stage: String,
    pub url: String,
    pub archive_sha256: String,
    pub binary: String,
    pub binary_sha256: String,
    pub destination: String,
}

fn path(value: &str, absolute: bool) -> Result<()> {
    ensure!(
        !value.is_empty() && value != "/",
        "image path must not be empty or `/`"
    );
    ensure!(
        value.starts_with('/') == absolute,
        "image path `{value}` must be {}",
        if absolute { "absolute" } else { "relative" }
    );
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
            && !value.split('/').any(|part| part == ".."),
        "image path `{value}` must use plain path characters without `..`"
    );
    Ok(())
}

fn name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            && value.as_bytes()[0].is_ascii_alphanumeric(),
        "invalid image binary or crate name `{value}`"
    );
    Ok(())
}

fn stage(value: &str, binaries: &[String]) -> Result<()> {
    ensure!(
        binaries.iter().any(|binary| binary == value),
        "image stage `{value}` is not a declared binary"
    );
    Ok(())
}

fn builder_source(value: &str) -> String {
    if value.starts_with('/') {
        value.to_owned()
    } else {
        format!("/workspace/{value}")
    }
}

pub(crate) fn validate(manifest: &Manifest) -> Result<()> {
    let image = &manifest.backend.image;
    if image.is_empty() {
        return Ok(());
    }
    ensure!(
        manifest.capabilities.backend,
        "backend.image requires capabilities.backend = true"
    );
    let binaries = validated_binaries(image, manifest.capabilities.worker)?;
    if let Some(value) = &image.bin_crate {
        name(value)?;
    }
    if let Some(value) = &image.backend_context {
        path(value, false)?;
    }
    ensure!(
        image.cargo_build_jobs != Some(0),
        "backend.image.cargo_build_jobs must be positive"
    );
    for input in &image.build_inputs {
        path(&input.source, false)?;
        path(&input.destination, true)?;
    }
    let outputs = validated_outputs(&image.pre_build)?;
    validate_runtime_files(&image.runtime_files, &binaries, &outputs)?;
    for file in &image.runtime_binaries {
        stage(&file.stage, &binaries)?;
        stage(&file.binary, &binaries)?;
    }
    for directory in &image.writable_directories {
        stage(&directory.stage, &binaries)?;
        path(&directory.path, true)?;
    }
    for download in &image.downloads {
        stage(&download.stage, &binaries)?;
        validate_download(download)?;
    }
    for package in &image.apt_packages {
        ensure!(
            !package.is_empty()
                && package.as_bytes()[0].is_ascii_alphanumeric()
                && package
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte)),
            "invalid builder apt package `{package}`"
        );
    }
    Ok(())
}

fn validated_binaries(image: &BackendImage, worker: bool) -> Result<Vec<String>> {
    let binaries = image.binaries(worker);
    ensure!(
        !binaries.is_empty(),
        "backend.image.binaries must not be empty"
    );
    let mut unique = BTreeSet::new();
    for binary in &binaries {
        name(binary)?;
        ensure!(
            binary != "builder" && !binary.starts_with("baukit-"),
            "image binary `{binary}` uses a reserved stage name"
        );
        ensure!(unique.insert(binary), "duplicate image binary `{binary}`");
    }
    Ok(binaries)
}

fn validated_outputs(steps: &[PreBuildStep]) -> Result<Vec<String>> {
    let mut outputs = Vec::new();
    for step in steps {
        ensure!(
            !step.command.is_empty() && !step.command[0].is_empty(),
            "image pre-build command must not be empty"
        );
        ensure!(
            step.command
                .iter()
                .all(|arg| !arg.contains(['\n', '\r', '\0'])),
            "image pre-build arguments must not contain newlines or NUL"
        );
        for output in &step.outputs {
            path(output, output.starts_with('/'))?;
            outputs.push(builder_source(output));
        }
    }
    Ok(outputs)
}

fn validate_runtime_files(
    files: &[RuntimeFile],
    binaries: &[String],
    outputs: &[String],
) -> Result<()> {
    for file in files {
        stage(&file.stage, binaries)?;
        path(&file.source, file.source.starts_with('/'))?;
        path(&file.destination, true)?;
        let source = builder_source(&file.source);
        ensure!(
            source == "/workspace"
                || source.starts_with("/workspace/")
                || outputs.iter().any(|output| source == *output
                    || source.starts_with(&format!("{}/", output.trim_end_matches('/')))),
            "runtime source `{}` outside /workspace must be a declared pre-build output",
            file.source
        );
    }
    Ok(())
}

fn validate_download(download: &Download) -> Result<()> {
    let host = download
        .url
        .strip_prefix("https://")
        .and_then(|url| url.split('/').next());
    ensure!(
        host.is_some_and(|host| !host.is_empty() && !host.contains('@'))
            && !download.url.chars().any(|c| c.is_whitespace()
                || c.is_control()
                || matches!(c, '"' | '\\' | '$' | '`')),
        "image download URL must use https with a host and no credentials or whitespace"
    );
    for checksum in [&download.archive_sha256, &download.binary_sha256] {
        ensure!(
            checksum.len() == 64 && checksum.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "image download checksum must contain 64 hexadecimal sha256 digits"
        );
    }
    path(&download.binary, false)?;
    path(&download.destination, true)?;
    Ok(())
}

pub fn render_dockerfile(manifest: &Manifest) -> Result<String> {
    ensure!(
        manifest.capabilities.backend,
        "Dockerfile generation requires capabilities.backend = true"
    );
    validate_name(&manifest.app.name)?;
    validate(manifest)?;
    let image = &manifest.backend.image;
    let commands = image
        .pre_build
        .iter()
        .map(|step| {
            step.command
                .iter()
                .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>();
    let files = image.runtime_files.iter().map(|file| context! {stage => &file.stage, source => builder_source(&file.source), destination => &file.destination}).collect::<Vec<_>>();
    let mut environment = Environment::new();
    environment.set_keep_trailing_newline(true);
    environment.render_str(include_str!("../../templates/backend/backend/Dockerfile.jinja"), context! {
        context => context! {
            app_name => &manifest.app.name,
            bin_crate => image.bin_crate.clone().unwrap_or_else(|| format!("{}-bin", manifest.app.name)),
            backend_context => image.backend_context.as_deref().unwrap_or("."),
            binaries => image.binaries(manifest.capabilities.worker),
            image => image, commands => commands, runtime_files => files,
        }
    }).context("could not render backend Dockerfile")
}

pub(crate) fn output_path(root: &Path, manifest: &Manifest) -> Result<PathBuf> {
    if let Some(relative) = &manifest.doctor.backend_dockerfile {
        return doctor_layout::product_path(root, relative);
    }
    Ok(doctor_layout::backend_manifest(root, manifest)?
        .parent()
        .context("backend manifest has no parent")?
        .join("Dockerfile"))
}

pub fn generate_dockerfile(root: &Path, check: bool) -> Result<PathBuf> {
    let manifest = read_manifest(root)?;
    let rendered = render_dockerfile(&manifest)?;
    let output = output_path(root, &manifest)?;
    if check {
        let existing = fs::read(&output).with_context(|| {
            format!(
                "missing or unreadable Dockerfile `{}`; run `baukit generate dockerfile`",
                output.display()
            )
        })?;
        if existing != rendered.as_bytes() {
            let line = existing
                .split(|byte| *byte == b'\n')
                .zip(rendered.as_bytes().split(|byte| *byte == b'\n'))
                .position(|(left, right)| left != right)
                .map_or_else(
                    || {
                        existing
                            .split(|byte| *byte == b'\n')
                            .count()
                            .min(rendered.lines().count())
                    },
                    |line| line + 1,
                );
            bail!(
                "Dockerfile `{}` differs from the render near line {line}; run `baukit generate dockerfile` and review `git diff -- {}`",
                output.strip_prefix(root)?.display(),
                output.strip_prefix(root)?.display()
            );
        }
    } else {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, rendered)
            .with_context(|| format!("could not write {}", output.display()))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(image: &str) -> Result<Manifest> {
        let source = format!(
            r#"
schema_version = 1
template_version = "0.10.8"
[app]
name = "test-product"
[capabilities]
backend = true
mobile = false
web = false
[dependencies.baukit]
source = "registry"
version = "0.10.8"
[openapi]
schema = "backend/openapi.json"
[backend.image]
{image}
"#
        );
        Ok(toml::from_str(&source)?)
    }

    #[test]
    fn defaults_and_worker_follow_capabilities() -> Result<()> {
        let mut manifest = manifest("")?;
        let output = render_dockerfile(&manifest)?;
        assert!(output.starts_with("# syntax=docker/dockerfile:1.28.0\n"));
        assert!(output.contains("ARG CARGO_BUILD_JOBS\n"));
        assert!(!output.contains("ENV CARGO_BUILD_JOBS"));
        assert!(output.contains("SQLX_OFFLINE=true"));
        assert!(output.contains("--bin api --bin migrate \\\n"));
        assert!(output.contains("COPY --from=builder /workspace/migrations /workspace/migrations"));
        assert!(output.contains("/workspace/crates/test-product-bin/Cargo.toml"));
        assert!(!output.contains("AS worker"));
        for (key, image) in [
            ("RUST_IMAGE", "rust:1.99.0-trixie"),
            ("RUNTIME_IMAGE", "gcr.io/distroless/cc-debian13:nonroot"),
        ] {
            let line = output
                .lines()
                .find(|line| line.starts_with(&format!("ARG {key}=")))
                .expect("base image argument");
            assert!(line.starts_with(&format!("ARG {key}={image}@sha256:")));
            assert_eq!(line.rsplit(':').next().expect("digest").len(), 64);
        }
        manifest.capabilities.worker = true;
        let output = render_dockerfile(&manifest)?;
        assert!(output.contains("--bin api --bin migrate --bin worker"));
        assert!(output.contains("FROM ${RUNTIME_IMAGE} AS worker"));
        assert!(!toml::to_string(&manifest)?.contains("[backend"));
        Ok(())
    }

    #[test]
    fn every_option_renders_from_the_manifest() -> Result<()> {
        let manifest = manifest(
            r#"
binaries = ["api", "migrate", "seed"]
bin_crate = "custom-bin"
backend_context = "backend"
cargo_build_jobs = 6
apt_packages = ["clang", "cmake", "libclang-dev", "pkg-config"]
build_inputs = [{ source = "content", destination = "/content/" }]
pre_build = [{ command = ["cargo", "run", "--locked", "--release", "-p", "compiler", "--", "build", "/content", "--out", "/generated/content"], outputs = ["/generated/content"] }]
runtime_files = [{ stage = "api", source = "/generated/content", destination = "/app/content" }, { stage = "migrate", source = "crates/postgres/Cargo.toml", destination = "/workspace/crates/postgres/Cargo.toml" }]
runtime_binaries = [{ stage = "migrate", binary = "seed" }]
writable_directories = [{ stage = "api", path = "/app/var/artifacts" }]
"#,
        )?;
        let output = render_dockerfile(&manifest)?;
        for expected in [
            "ARG BACKEND_CONTEXT=backend",
            "ARG CARGO_BUILD_JOBS=6",
            "ARG BAUKIT_CONTEXT=.",
            "ARG BAUKIT_DESTINATION=/tmp/unused-baukit-context",
            "ARG LIMITS_FILE=limits.json",
            "ARG GIT_COMMIT=unknown",
            "apt-get install --no-install-recommends --yes clang cmake libclang-dev pkg-config",
            "COPY [\"content\", \"/content/\"]",
            "'cargo' 'run' '--locked' '--release' '-p' 'compiler' '--' 'build' '/content' '--out' '/generated/content' \\\n    && cargo build",
            "--bin api --bin migrate --bin seed",
            "-p custom-bin",
            "COPY --from=builder [\"/generated/content\", \"/app/content\"]",
            "COPY --from=builder [\"/workspace/crates/postgres/Cargo.toml\", \"/workspace/crates/postgres/Cargo.toml\"]",
            "COPY --from=builder /out/seed /app/seed",
            "COPY --from=builder --chown=nonroot:nonroot /out/directories/0 /app/var/artifacts",
            "FROM ${RUNTIME_IMAGE} AS seed",
            "ENTRYPOINT [\"/app/seed\"]",
        ] {
            assert!(output.contains(expected), "missing {expected}\n{output}");
        }
        assert!(!output.contains("AS worker"));
        Ok(())
    }

    #[test]
    fn downloads_verify_archive_and_binary_and_copy_as_root() -> Result<()> {
        let manifest = manifest(&format!(
            r#"
downloads = [{{ stage = "api", url = "https://example.com/tool.tar.gz", archive_sha256 = "{}", binary = "tool", binary_sha256 = "{}", destination = "/app/tool" }}]
"#,
            "a".repeat(64),
            "b".repeat(64)
        ))?;
        let output = render_dockerfile(&manifest)?;
        assert!(output.contains(&format!(
            "ADD --checksum=sha256:{} https://example.com/tool.tar.gz /tmp/release.tar.gz",
            "a".repeat(64)
        )));
        assert!(output.contains("tar -xzf /tmp/release.tar.gz -C /out -- tool"));
        assert!(output.contains(&format!(
            "echo '{}  /out/tool' | sha256sum --check",
            "b".repeat(64)
        )));
        assert!(output.contains(
            "COPY --from=baukit-download-0 --chown=0:0 --chmod=0555 /out/tool /app/tool"
        ));
        Ok(())
    }

    #[test]
    fn pre_build_quotes_arguments_and_accepts_output_children() -> Result<()> {
        let manifest = manifest(
            r#"
pre_build = [{ command = ["printf", "it's $HOME; $(false)"], outputs = ["/generated"] }]
runtime_files = [{ stage = "api", source = "/generated/content/file", destination = "/app/file" }, { stage = "api", source = "/workspace", destination = "/app/workspace" }]
"#,
        )?;
        let output = render_dockerfile(&manifest)?;
        assert!(output.contains("'printf' 'it'\\''s $HOME; $(false)'"));
        assert!(output.contains("COPY --from=builder [\"/workspace\", \"/app/workspace\"]"));
        Ok(())
    }

    #[test]
    fn rejects_each_invalid_image_declaration() -> Result<()> {
        let download = |field: &str, value: &str| {
            format!(
                r#"downloads = [{{stage = "{}", url = "{}", archive_sha256 = "{}", binary = "{}", binary_sha256 = "{}", destination = "{}"}}]"#,
                if field == "stage" { value } else { "api" },
                if field == "url" {
                    value
                } else {
                    "https://example.com/tool.tar.gz"
                },
                if field == "archive_sha256" {
                    value.to_owned()
                } else {
                    "a".repeat(64)
                },
                if field == "binary" { value } else { "tool" },
                if field == "binary_sha256" {
                    value.to_owned()
                } else {
                    "b".repeat(64)
                },
                if field == "destination" {
                    value
                } else {
                    "/app/tool"
                }
            )
        };
        let mut cases = vec![
            ("binaries = []".to_owned(), "must not be empty"),
            ("binaries = [\"api\", \"api\"]".to_owned(), "duplicate"),
            ("binaries = [\"../api\"]".to_owned(), "invalid image"),
            ("binaries = [\"builder\"]".to_owned(), "reserved"),
            ("binaries = [\"baukit-download-0\"]".to_owned(), "reserved"),
            ("bin_crate = \"bad;crate\"".to_owned(), "invalid image"),
            ("backend_context = \"/backend\"".to_owned(), "relative"),
            ("backend_context = \"../backend\"".to_owned(), "without `..`"),
            ("cargo_build_jobs = 0".to_owned(), "positive"),
            ("build_inputs = [{source = \"/content\", destination = \"/content\"}]".to_owned(), "relative"),
            ("build_inputs = [{source = \"../content\", destination = \"/content\"}]".to_owned(), "without `..`"),
            ("build_inputs = [{source = \"content\", destination = \"content\"}]".to_owned(), "absolute"),
            ("build_inputs = [{source = \"content\", destination = \"/../content\"}]".to_owned(), "without `..`"),
            ("pre_build = [{command = []}]".to_owned(), "command must not be empty"),
            ("pre_build = [{command = [\"\"]}]".to_owned(), "command must not be empty"),
            ("pre_build = [{command = [\"line\\nline\"]}]".to_owned(), "newlines or NUL"),
            ("pre_build = [{command = [\"true\"], outputs = [\"../file\"]}]".to_owned(), "without `..`"),
            ("runtime_files = [{stage = \"absent\", source = \"file\", destination = \"/app/file\"}]".to_owned(), "not a declared binary"),
            ("runtime_files = [{stage = \"api\", source = \"/outside/file\", destination = \"/app/file\"}]".to_owned(), "declared pre-build output"),
            ("runtime_files = [{stage = \"api\", source = \"/workspace/../file\", destination = \"/app/file\"}]".to_owned(), "without `..`"),
            ("runtime_files = [{stage = \"api\", source = \"file\", destination = \"file\"}]".to_owned(), "absolute"),
            ("runtime_binaries = [{stage = \"absent\", binary = \"api\"}]".to_owned(), "not a declared binary"),
            ("runtime_binaries = [{stage = \"api\", binary = \"absent\"}]".to_owned(), "not a declared binary"),
            ("writable_directories = [{stage = \"absent\", path = \"/app/files\"}]".to_owned(), "not a declared binary"),
            ("writable_directories = [{stage = \"api\", path = \"files\"}]".to_owned(), "absolute"),
            ("writable_directories = [{stage = \"api\", path = \"/app/../files\"}]".to_owned(), "without `..`"),
            ("apt_packages = [\"--bad\"]".to_owned(), "invalid builder apt"),
            ("apt_packages = [\"clang;false\"]".to_owned(), "invalid builder apt"),
            ("build_inputs = [{source = \"\", destination = \"/app/file\"}]".to_owned(), "must not be empty"),
            ("build_inputs = [{source = \"$FILE\", destination = \"/app/file\"}]".to_owned(), "plain path"),
        ];
        for (field, value, expected) in [
            ("stage", "absent", "not a declared binary"),
            ("url", "http://example.com/tool", "https"),
            ("url", "https:///tool", "https"),
            ("url", "https://user:pass@example.com/tool", "https"),
            ("url", "https://example.com/bad url", "https"),
            ("archive_sha256", "abcd", "64 hexadecimal"),
            ("binary_sha256", &"z".repeat(64), "64 hexadecimal"),
            ("binary", "../tool", "without `..`"),
            ("binary", "/tool", "relative"),
            ("destination", "tool", "absolute"),
        ] {
            cases.push((download(field, value), expected));
        }
        for (source, expected) in cases {
            let error = render_dockerfile(&manifest(&source)?)
                .expect_err(&source)
                .to_string();
            assert!(error.contains(expected), "{source}: {error}");
        }
        let mut frontend = manifest("cargo_build_jobs = 6")?;
        frontend.capabilities.backend = false;
        assert!(
            validate(&frontend)
                .expect_err("frontend image")
                .to_string()
                .contains("capabilities.backend")
        );
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields_in_every_image_table() -> Result<()> {
        for source in [
            "typo = true",
            "build_inputs = [{ source = 'file', destination = '/file', typo = true }]",
            "pre_build = [{command = ['true'], typo = true}]",
            "runtime_files = [{stage = 'api', source = 'file', destination = '/file', typo = true}]",
            "runtime_binaries = [{stage = 'api', binary = 'migrate', typo = true}]",
            "writable_directories = [{stage = 'api', path = '/files', typo = true}]",
            "downloads = [{stage = 'api', url = 'https://example.com/tool', archive_sha256 = 'a', binary = 'tool', binary_sha256 = 'b', destination = '/tool', typo = true}]",
        ] {
            assert!(
                manifest(source)
                    .expect_err(source)
                    .to_string()
                    .contains("unknown field")
            );
        }
        assert!(
            toml::from_str::<BackendManifest>("typo = true")
                .expect_err("backend table typo")
                .to_string()
                .contains("unknown field")
        );
        Ok(())
    }
}
