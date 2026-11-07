use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use minijinja::{Environment, context};
use serde::{Deserialize, Serialize};

use crate::{Manifest, doctor_layout, generate::validate_name, read_manifest};

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
    pub runtime_packages: Vec<RuntimePackages>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<ImageVariant>,
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
pub struct RuntimePackages {
    pub stage: String,
    pub packages: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ImageVariant {
    pub name: String,
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runtime_packages: Vec<RuntimePackages>,
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
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric),
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
        manifest.capabilities.components.backend(),
        "backend.image requires capabilities.backend = true"
    );
    let binaries = validated_binaries(image, manifest.capabilities.components.worker())?;
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
    validate_runtime_packages(&image.runtime_packages, &binaries)?;
    validate_variants(&image.variants, &binaries)?;
    for directory in &image.writable_directories {
        stage(&directory.stage, &binaries)?;
        path(&directory.path, true)?;
    }
    for download in &image.downloads {
        stage(&download.stage, &binaries)?;
        validate_download(download)?;
    }
    for package in &image.apt_packages {
        apt_package(package, "builder")?;
    }
    Ok(())
}

fn apt_package(package: &str, scope: &str) -> Result<()> {
    ensure!(
        package
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && package
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte)),
        "invalid {scope} apt package `{package}`"
    );
    Ok(())
}

fn validate_runtime_packages(entries: &[RuntimePackages], binaries: &[String]) -> Result<()> {
    let mut stages = BTreeSet::new();
    for entry in entries {
        stage(&entry.stage, binaries)?;
        ensure!(
            stages.insert(&entry.stage),
            "duplicate runtime packages stage `{}`",
            entry.stage
        );
        for package in &entry.packages {
            apt_package(package, "runtime")?;
        }
    }
    Ok(())
}

fn validate_variants(variants: &[ImageVariant], binaries: &[String]) -> Result<()> {
    let mut names = BTreeSet::new();
    let mut stages = binaries.iter().cloned().collect::<BTreeSet<_>>();
    stages.insert("builder".to_owned());
    for variant in variants {
        ensure!(
            variant
                .name
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
                && variant
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "invalid image variant name `{}`; expected [a-z][a-z0-9-]*",
            variant.name
        );
        ensure!(
            names.insert(&variant.name),
            "duplicate image variant `{}`",
            variant.name
        );
        ensure!(
            !variant.features.is_empty(),
            "image variant `{}` must have at least one feature",
            variant.name
        );
        validate_features(&variant.features)?;
        validate_runtime_packages(&variant.runtime_packages, binaries)?;
        for binary in std::iter::once("builder").chain(binaries.iter().map(String::as_str)) {
            let name = format!("{binary}-{}", variant.name);
            ensure!(
                stages.insert(name.clone()),
                "image variant `{}` produces duplicate image stage `{name}`",
                variant.name
            );
        }
    }
    Ok(())
}

fn validate_features(features: &[String]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for feature in features {
        ensure!(
            !feature.is_empty()
                && feature
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-+./".contains(&byte)),
            "invalid cargo feature `{feature}`"
        );
        ensure!(
            unique.insert(feature),
            "duplicate cargo feature `{feature}`"
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
            step.command
                .first()
                .is_some_and(|program| !program.is_empty()),
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

#[derive(Serialize)]
struct BuilderStage {
    name: String,
    target_cache_id: String,
    features: Vec<String>,
}

fn builder_stages(image: &BackendImage) -> Vec<BuilderStage> {
    let mut builders = vec![BuilderStage {
        name: "builder".to_owned(),
        target_cache_id: "${CARGO_TARGET_CACHE_PREFIX}-target".to_owned(),
        features: Vec::new(),
    }];
    builders.extend(image.variants.iter().map(|variant| BuilderStage {
        name: format!("builder-{}", variant.name),
        target_cache_id: format!("${{CARGO_TARGET_CACHE_PREFIX}}-target-{}", variant.name),
        features: variant.features.clone(),
    }));
    builders
}

#[derive(Serialize)]
struct RuntimeStage {
    name: String,
    binary: String,
    builder: String,
    slim: bool,
    packages: Vec<String>,
}

fn stage_packages<'a>(entries: &'a [RuntimePackages], binary: &str) -> Option<&'a [String]> {
    entries
        .iter()
        .find(|entry| entry.stage == binary)
        .map(|entry| entry.packages.as_slice())
}

fn runtime_stage(
    image: &BackendImage,
    binary: &str,
    variant: Option<&ImageVariant>,
) -> RuntimeStage {
    let base_packages = stage_packages(&image.runtime_packages, binary);
    let Some(variant) = variant else {
        return RuntimeStage {
            name: binary.to_owned(),
            binary: binary.to_owned(),
            builder: "builder".to_owned(),
            slim: base_packages.is_some(),
            packages: base_packages.unwrap_or_default().to_vec(),
        };
    };
    let variant_packages = stage_packages(&variant.runtime_packages, binary);
    let mut unique = BTreeSet::new();
    let packages = base_packages
        .unwrap_or_default()
        .iter()
        .chain(variant_packages.unwrap_or_default())
        .filter(|package| unique.insert(*package))
        .cloned()
        .collect();
    RuntimeStage {
        name: format!("{binary}-{}", variant.name),
        binary: binary.to_owned(),
        builder: format!("builder-{}", variant.name),
        slim: base_packages.is_some() || variant_packages.is_some(),
        packages,
    }
}

fn runtime_stages(image: &BackendImage, binaries: &[String]) -> Vec<RuntimeStage> {
    let mut stages = binaries
        .iter()
        .map(|binary| runtime_stage(image, binary, None))
        .collect::<Vec<_>>();
    for variant in &image.variants {
        stages.extend(
            binaries
                .iter()
                .map(|binary| runtime_stage(image, binary, Some(variant))),
        );
    }
    stages
}

/// Renders the backend Dockerfile described by `manifest`.
///
/// # Errors
///
/// Fails when the manifest has no backend capability, the app name is invalid, or
/// the backend image section does not validate.
pub fn render_dockerfile(manifest: &Manifest) -> Result<String> {
    ensure!(
        manifest.capabilities.components.backend(),
        "Dockerfile generation requires capabilities.backend = true"
    );
    validate_name(&manifest.app.name)?;
    validate(manifest)?;
    let image = &manifest.backend.image;
    let binaries = image.binaries(manifest.capabilities.components.worker());
    let builders = builder_stages(image);
    let stages = runtime_stages(image, &binaries);
    let packages_runtime = stages.iter().any(|stage| stage.slim);
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
            binaries => binaries, builders => builders, stages => stages,
            packages_runtime => packages_runtime,
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

/// Writes the backend Dockerfile, or with `check` compares it to the committed file.
///
/// # Errors
///
/// Fails when `baukit.toml` cannot be read or rendered, the backend manifest path
/// is invalid, or the file cannot be written. With `check`, also fails when the
/// Dockerfile is missing, unreadable, or differs from the rendered output.
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
mod tests;
