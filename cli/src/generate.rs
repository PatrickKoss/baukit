mod dependencies;
mod manifest_source;
mod render;
mod template_files;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    LAST_TEMPLATE_MIGRATION_VERSION, MAX_APPLICATION_NAME_LEN, PLATFORM_MIGRATION_PATH,
    REMOTE_MCP_CLIENT_SCOPES, TEMPLATE_VERSION,
    check::TEMPLATE_FILES_MANIFEST,
    components::Components,
    doctor_layout,
    generate::dependencies::dependency_context,
    generate::render::{
        AuthContext, DeclaredPorts, RENDERED_SYMLINKS, TemplateContext, product_description,
        render_product,
    },
    manifest::{AuthProvider, Manifest, QualityProfile},
    openapi_client::{command_exists, run_checked},
    ports::{self, PortConfiguration},
};

#[derive(Clone, Debug)]
pub struct NewOptions {
    pub name: String,
    pub directory: PathBuf,
    pub components: Components,
    pub auth: Option<AuthProvider>,
    pub force: bool,
    pub into_existing: bool,
    pub resolve_lockfiles: bool,
    pub baukit_path: Option<PathBuf>,
    pub port_offset: u32,
    pub quality: QualityProfile,
}

pub fn generate_new(options: &NewOptions) -> Result<PathBuf> {
    validate_name(&options.name)?;
    validate_component_selection(options)?;
    let destination = if options.into_existing {
        options.directory.clone()
    } else {
        options.directory.join(&options.name)
    };
    let (ports, existing_manifest) = ports::generation_ports(&destination, options.port_offset)?;
    let manifest = existing_manifest
        .as_deref()
        .map(toml::from_str::<Manifest>)
        .transpose()?;
    let platform_migration = resolve_platform_migration(&destination, manifest.as_ref(), options)?;
    ensure_destination_accepts_output(&destination, options)?;

    let context = template_context(options, &ports, platform_migration)?;
    let mut rendered = render_product(&context, options)?;
    if let Some(source) = existing_manifest {
        rendered.insert(PathBuf::from("baukit.toml"), source.into_bytes());
    }
    write_rendered(&destination, rendered)?;

    if options.resolve_lockfiles {
        resolve_lockfiles(&destination, options)?;
    }

    Ok(destination)
}

fn validate_component_selection(options: &NewOptions) -> Result<()> {
    let components = options.components;
    if components.worker() && !components.backend() {
        bail!("--worker requires --backend because it is generated in the backend workspace");
    }
    if components.pwa() && !components.mobile() && !components.web() {
        bail!("--pwa requires --mobile or --web to serve the worker");
    }
    if components.platform() && !components.backend() {
        bail!("--platform requires --backend because the receiver uses PostgreSQL");
    }
    validate_mcp_options(options)?;
    if !components.backend() && !components.mobile() && !components.web() {
        bail!("select at least one capability: --backend, --mobile, or --web");
    }
    Ok(())
}

fn resolve_platform_migration(
    destination: &Path,
    manifest: Option<&Manifest>,
    options: &NewOptions,
) -> Result<String> {
    if !options.components.platform() {
        return Ok(PLATFORM_MIGRATION_PATH.to_owned());
    }
    let platform_migration = platform_migration_path(destination, manifest)?;
    doctor_layout::product_path(destination, &platform_migration)?;
    if Path::new(&platform_migration)
        .extension()
        .is_none_or(|extension| extension != "sql")
    {
        bail!("platform.migration must name a forward SQL migration");
    }
    Ok(platform_migration)
}

fn ensure_destination_accepts_output(destination: &Path, options: &NewOptions) -> Result<()> {
    let non_empty = destination.exists()
        && fs::read_dir(destination)
            .with_context(|| format!("could not inspect {}", destination.display()))?
            .next()
            .transpose()?
            .is_some();
    if non_empty && !options.force && !options.into_existing {
        bail!(
            "destination {} is not empty; choose an empty directory or pass --force to add only non-conflicting files",
            destination.display()
        );
    }
    Ok(())
}

fn template_context(
    options: &NewOptions,
    ports: &PortConfiguration,
    platform_migration: String,
) -> Result<TemplateContext> {
    let dependency = dependency_context(options)?;
    Ok(TemplateContext {
        app_name: options.name.clone(),
        app_crate: options.name.replace('-', "_"),
        app_env: options.name.replace('-', "_").to_ascii_uppercase(),
        template_version: TEMPLATE_VERSION.to_owned(),
        baukit_dependencies: dependency.cargo,
        baukit_web_typescript_dependencies: dependency.web_typescript,
        baukit_web_typescript_dev_dependencies: dependency.web_dev_typescript,
        baukit_mobile_typescript_dependencies: dependency.mobile_typescript,
        baukit_pnpm_overrides: dependency.pnpm_overrides,
        baukit_manifest: dependency.manifest,
        baukit_dependency_description: dependency.description,
        baukit_typescript_dependency_description: dependency.typescript_description,
        product_description: product_description(options),
        components: options.components,
        platform_migration,
        remote_mcp_client_scopes: REMOTE_MCP_CLIENT_SCOPES,
        auth: AuthContext(options.auth),
        quality_strict: options.quality == QualityProfile::Strict,
        port_offset: ports.offset,
        postgres_host_port: ports.postgres,
        api_host_port: ports.api,
        ops_host_port: ports.ops,
        keycloak_host_port: ports.keycloak,
        fake_provider_host_port: ports.fake_provider,
        redis_host_port: ports.redis,
        web_port: ports.web.unwrap_or(ports::WEB_PORT),
        web_preview_port: ports.web_preview.unwrap_or(ports::WEB_PREVIEW_PORT),
        metro_port: ports.metro,
        local_callback_port: ports.local_callback_port(),
        declared_ports: DeclaredPorts {
            dev: ports.web.is_some() || ports.web_preview.is_some() || ports.metro.is_some(),
            web_preview: ports.web_preview.is_some(),
            service: ports.service_ports_declared,
        },
        api_container_port: ports.api_container,
        ops_container_port: ports.ops_container,
    })
}

/// Writes new files and reports existing files whose content differs as conflicts. The
/// template-files manifest is derived from the other files, so it is always rewritten.
fn write_rendered(destination: &Path, rendered: BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    let mut conflicts = Vec::new();
    fs::create_dir_all(destination)
        .with_context(|| format!("could not create {}", destination.display()))?;
    for (relative, bytes) in rendered {
        let output = destination.join(&relative);
        if output.exists() && relative != Path::new(TEMPLATE_FILES_MANIFEST) {
            let existing = fs::read(&output)
                .with_context(|| format!("could not read existing file {}", output.display()))?;
            if existing != bytes {
                conflicts.push(output.strip_prefix(destination)?.to_path_buf());
            }
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, bytes)
            .with_context(|| format!("could not write {}", output.display()))?;
    }
    for (link, target) in RENDERED_SYMLINKS {
        if !write_symlink(destination, Path::new(link), Path::new(target))? {
            conflicts.push(PathBuf::from(link));
        }
    }
    if conflicts.is_empty() {
        return Ok(());
    }
    conflicts.sort();
    let mut report = String::from(
        "baukit generation conflicts\n\nThe following existing files differ from the template and were not overwritten:\n",
    );
    for conflict in &conflicts {
        report.push_str("- ");
        report.push_str(&conflict.display().to_string());
        report.push('\n');
    }
    let conflict_path = available_conflict_path(destination)?;
    fs::write(&conflict_path, report)?;
    bail!(
        "generation found {} conflict(s); no modified file was overwritten. See {}",
        conflicts.len(),
        conflict_path.display()
    )
}

/// Creates `link` pointing at `target`; returns false when a different file already holds the path.
fn write_symlink(destination: &Path, link: &Path, target: &Path) -> Result<bool> {
    let output = destination.join(link);
    match fs::symlink_metadata(&output) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink() && fs::read_link(&output)? == target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_symlink(target, &output)?;
            Ok(true)
        }
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", output.display())),
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, output: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, output)
        .with_context(|| format!("could not create symlink {}", output.display()))
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, output: &Path) -> Result<()> {
    bail!(
        "creating the symlink {} needs a Unix host",
        output.display()
    )
}

pub(crate) fn resolve_lockfiles(destination: &Path, options: &NewOptions) -> Result<()> {
    if options.components.backend() && !destination.join("backend/Cargo.lock").is_file() {
        run_checked(
            Command::new("cargo").current_dir(destination).args([
                "generate-lockfile",
                "--manifest-path",
                "backend/Cargo.toml",
            ]),
            "Cargo lockfile generation",
        )?;
    }

    for capability in [
        (options.components.web(), "web"),
        (options.components.mobile(), "mobile"),
    ]
    .into_iter()
    .filter_map(|(enabled, name)| enabled.then_some(name))
    {
        if destination
            .join(capability)
            .join("pnpm-lock.yaml")
            .is_file()
        {
            continue;
        }
        if !command_exists("corepack") {
            bail!(
                "generated source files, but {capability}/pnpm-lock.yaml needs current Node.js LTS with corepack; install Node.js, then run `sh scripts/lockfiles.sh`"
            );
        }
        run_checked(
            Command::new("corepack")
                .current_dir(destination.join(capability))
                .args([
                    "pnpm@12.9.1",
                    "install",
                    "--lockfile-only",
                    "--ignore-scripts",
                ]),
            &format!("{capability} pnpm lockfile generation"),
        )?;
    }
    Ok(())
}

pub(crate) fn available_conflict_path(destination: &Path) -> Result<PathBuf> {
    for index in 0..100 {
        let name = if index == 0 {
            "baukit-conflicts.txt".to_owned()
        } else {
            format!("baukit-conflicts.{index}.txt")
        };
        let path = destination.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    bail!(
        "could not write a conflict report in {}; baukit-conflicts.txt through baukit-conflicts.99.txt already exist",
        destination.display()
    )
}

pub(crate) fn platform_migration_path(
    destination: &Path,
    manifest: Option<&Manifest>,
) -> Result<String> {
    if let Some(platform) = manifest.and_then(|manifest| manifest.platform.as_ref()) {
        return Ok(platform.migration.clone());
    }
    let directory = manifest
        .and_then(|manifest| manifest.doctor.migrations.as_deref())
        .unwrap_or("backend/migrations");
    let path = doctor_layout::product_path(destination, directory)?;
    let mut version = LAST_TEMPLATE_MIGRATION_VERSION;
    if path.exists() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|extension| extension != "sql") {
                continue;
            }
            if let Some(number) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.split('_').next())
                .and_then(|prefix| prefix.parse::<u64>().ok())
            {
                version = version.max(number);
            }
        }
    }
    let next = version
        .checked_add(1)
        .ok_or_else(|| anyhow!("no platform migration number remains after {version}"))?;
    Ok(format!("{directory}/{next:04}_platform.sql"))
}

pub(crate) fn validate_mcp_options(options: &NewOptions) -> Result<()> {
    if !options.components.mcp() {
        return Ok(());
    }
    if !options.components.backend() {
        bail!("--mcp requires --backend because the Rust server runs in the backend workspace");
    }
    if options.auth.is_none() {
        bail!("--mcp requires --auth oidc, clerk, or workos");
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.len() > MAX_APPLICATION_NAME_LEN {
        bail!(
            "invalid application name `{name}`; maximum length is {MAX_APPLICATION_NAME_LEN} ASCII characters because generated Kubernetes Service names append `-baukit-app-worker-ops` and must fit the 63-character limit"
        );
    }
    let valid = !name.is_empty()
        && name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.contains("--");
    if valid {
        Ok(())
    } else {
        bail!(
            "invalid application name `{name}`; use 1-{MAX_APPLICATION_NAME_LEN} lowercase ASCII letters, digits, and single hyphens, starting with a letter"
        )
    }
}
