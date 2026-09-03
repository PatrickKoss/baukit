use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use include_dir::{Dir, include_dir};
use minijinja::{Environment, context};
use serde::{Deserialize, Serialize};

static BACKEND_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/backend");
static COMMON_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/common");
static MOBILE_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/mobile");
static WEB_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/web");
static WORKER_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/worker");

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const TEMPLATE_VERSION: &str = include_str!("../../templates/VERSION").trim_ascii();

const POSTGRES_HOST_PORT: u32 = 5432;
const API_HOST_PORT: u32 = 8080;
const OPS_HOST_PORT: u32 = 9090;
const KEYCLOAK_HOST_PORT: u32 = 8081;
const FAKE_PROVIDER_HOST_PORT: u32 = 18081;
const OPENAPI_TYPESCRIPT_PACKAGE: &str = "openapi-typescript@7.13.0";

const EXPECTED_BACKEND_FILES: &[&str] = &[
    "README.md",
    "Makefile",
    ".dockerignore",
    ".cargo/config.toml",
    "compose.yaml",
    "deploy/values.yaml",
    ".agents/skills/baukit-backend/SKILL.md",
    ".claude/skills/baukit-backend/SKILL.md",
    "scripts/openapi.sh",
    "scripts/openapi-client.sh",
    "docs/fake-providers.md",
    "docs/openapi-drift.md",
    "docs/syncable-tables.md",
    "backend/Cargo.toml",
    "backend/.dockerignore",
    "backend/Dockerfile",
    "backend/crates/__APP__-domain/Cargo.toml",
    "backend/crates/__APP__-domain/src/limits.rs",
    "backend/crates/__APP__-ports/Cargo.toml",
    "backend/crates/__APP__-services/Cargo.toml",
    "backend/crates/__APP__-api/Cargo.toml",
    "backend/crates/__APP__-postgres/Cargo.toml",
    "backend/crates/__APP__-bin/Cargo.toml",
];

const EXPECTED_WORKER_FILES: &[&str] = &[
    "backend/crates/__APP__-worker/Cargo.toml",
    "backend/crates/__APP__-worker/src/lib.rs",
    "backend/crates/__APP__-bin/src/bin/worker.rs",
    "backend/tests/worker_integration.rs",
];

const EXPECTED_COMMON_FILES: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    ".github/workflows/ci.yml",
    "limits.json",
    "docs/navigation-recipe.md",
    "docs/observability-lint.md",
    "docs/resource-budgets.md",
    "scripts/lockfiles.sh",
    "scripts/preflight.sh",
];

const EXPECTED_STRICT_FILES: &[&str] = &["scripts/quality-gate.sh"];

const EXPECTED_STRICT_BACKEND_FILES: &[&str] = &[
    "scripts/check-migrations-immutable.sh",
    "scripts/check-migrations-immutable.test.sh",
];

const EXPECTED_MOBILE_FILES: &[&str] = &[
    "mobile/package.json",
    "mobile/pnpm-workspace.yaml",
    "mobile/app.config.ts",
    "mobile/tsconfig.json",
    "mobile/metro.config.js",
    "mobile/eslint.config.js",
    "mobile/jest.config.cjs",
    "mobile/app/_layout.tsx",
    "mobile/app/(tabs)/_layout.tsx",
    "mobile/app/(tabs)/index.tsx",
    "mobile/scripts/generate-tokens.mjs",
    "mobile/src/action-button.tsx",
    "mobile/src/app-shell.tsx",
    "mobile/src/api.ts",
    "mobile/src/analytics.ts",
    "mobile/src/back-or-replace.ts",
    "mobile/src/back-or-replace.test.ts",
    "mobile/src/localization/catalogs.test.ts",
    "mobile/src/localization/de.ts",
    "mobile/src/localization/en.ts",
    "mobile/src/localization/i18n.ts",
    "mobile/src/limits.ts",
    "mobile/src/limits.test.ts",
    "mobile/src/route-heading-focus.ts",
    "mobile/src/route-heading-focus.test.ts",
    "mobile/src/theme.ts",
    "mobile/src/tokens.ts",
    "mobile/src/record-store.ts",
];

const EXPECTED_WEB_FILES: &[&str] = &[
    "web/package.json",
    "web/pnpm-workspace.yaml",
    "web/index.html",
    "web/vite.config.ts",
    "web/tsconfig.json",
    "web/eslint.config.js",
    "web/vitest.config.ts",
    "web/src/App.tsx",
    "web/src/api.ts",
    "web/src/analytics.ts",
    "web/src/limits.ts",
    "web/src/limits.test.ts",
    "web/src/tokens.css",
    "web/e2e/playwright.config.ts",
    "web/e2e/qa.config.ts",
    "web/e2e/tsconfig.json",
    "web/e2e/tests/qa.ts",
    "web/e2e/tests/geometry.ts",
    "web/e2e/tests/console-warnings.ts",
    "web/e2e/tests/qa-axe.spec.ts",
    "web/e2e/tests/qa-keyboard.spec.ts",
    "web/e2e/tests/qa-overlay-dismiss.spec.ts",
    "web/e2e/tests/qa-submit-guards.spec.ts",
    "web/e2e/tests/qa-route-state.spec.ts",
    "web/e2e/tests/qa-auth-expiry.spec.ts",
    "web/e2e/tests/qa-auth-isolation.spec.ts",
    "web/e2e/tests/qa-scroll.spec.ts",
    "web/e2e/tests/qa-geometry.spec.ts",
    "web/e2e/tests/qa-console.spec.ts",
];

const EXPECTED_AUTH_BACKEND_FILES: &[&str] = &[
    "keycloak/realm.json",
    "backend/tests/auth_conformance.rs",
    "scripts/pkce-login.py",
];

const EXPECTED_AUTH_MOBILE_FILES: &[&str] = &[
    "mobile/app/(auth)/_layout.tsx",
    "mobile/app/(auth)/sign-in.tsx",
    "mobile/src/auth.ts",
    "mobile/src/auth.test.ts",
    "mobile/src/local-data.ts",
    "mobile/src/persistence-lifecycle.ts",
    "mobile/docs/local-data-retention.md",
];

const EXPECTED_AUTH_WEB_FILES: &[&str] = &[
    "web/src/auth.ts",
    "web/src/auth.test.ts",
    "web/src/local-data.ts",
    "web/src/persistence-lifecycle.ts",
    "web/docs/local-data-retention.md",
];

const EXPECTED_TYPESCRIPT_DEPENDENCIES: &[&str] = &[
    "@baukit/a11y-core",
    "@baukit/analytics-core",
    "@baukit/api-runtime",
    "@baukit/ui-tokens",
];

const EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES: &[&str] = &[
    "@baukit/data-contracts",
    "@baukit/data-contracts-expo-sqlite",
    "@baukit/localization-core",
    "@baukit/preferences-core",
];

const EXPECTED_MOBILE_AUTH_DEPENDENCIES: &[&str] = &["@baukit/auth-native"];
const EXPECTED_WEB_AUTH_DEPENDENCIES: &[&str] = &["@baukit/auth-web", "@baukit/data-contracts"];

#[derive(Clone, Debug)]
pub struct NewOptions {
    pub name: String,
    pub directory: PathBuf,
    pub backend: bool,
    pub worker: bool,
    pub mobile: bool,
    pub web: bool,
    pub auth: Option<AuthProvider>,
    pub force: bool,
    pub into_existing: bool,
    pub resolve_lockfiles: bool,
    pub baukit_path: Option<PathBuf>,
    pub port_offset: u32,
    pub quality: QualityProfile,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Oidc,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    #[default]
    Standard,
    Strict,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub template_version: String,
    #[serde(default)]
    pub port_offset: u32,
    pub app: AppManifest,
    #[serde(default)]
    pub quality: QualityManifest,
    pub capabilities: Capabilities,
    pub dependencies: Dependencies,
    pub openapi: OpenApiPaths,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppManifest {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QualityManifest {
    #[serde(default)]
    pub profile: QualityProfile,
    #[serde(default = "default_backend_coverage_lines")]
    pub backend_coverage_lines: u8,
    #[serde(default)]
    pub critical_paths: Vec<String>,
    #[serde(default = "default_webkit_repeats")]
    pub webkit_repeats: u8,
    #[serde(default)]
    pub full_stack_e2e: bool,
}

impl Default for QualityManifest {
    fn default() -> Self {
        Self {
            profile: QualityProfile::Standard,
            backend_coverage_lines: default_backend_coverage_lines(),
            critical_paths: Vec::new(),
            webkit_repeats: default_webkit_repeats(),
            full_stack_e2e: false,
        }
    }
}

const fn default_backend_coverage_lines() -> u8 {
    70
}

const fn default_webkit_repeats() -> u8 {
    3
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Capabilities {
    pub backend: bool,
    #[serde(default)]
    pub worker: bool,
    pub mobile: bool,
    pub web: bool,
    #[serde(default)]
    pub pwa: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthProvider>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dependencies {
    pub baukit: BaukitDependency,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum BaukitDependency {
    Path { path: String },
    Git { git: String, tag: String },
    Registry { version: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenApiPaths {
    pub schema: String,
    #[serde(default)]
    pub consumers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typescript: Option<String>,
}

impl OpenApiPaths {
    pub fn consumers(&self) -> Vec<&str> {
        if self.consumers.is_empty() {
            self.typescript.iter().map(String::as_str).collect()
        } else {
            self.consumers.iter().map(String::as_str).collect()
        }
    }
}

#[derive(Debug, Serialize)]
struct TemplateContext {
    app_name: String,
    app_crate: String,
    app_env: String,
    template_version: String,
    baukit_dependencies: String,
    baukit_web_typescript_dependencies: String,
    baukit_mobile_typescript_dependencies: String,
    baukit_manifest: String,
    baukit_dependency_description: String,
    baukit_typescript_dependency_description: String,
    product_description: String,
    backend: bool,
    worker: bool,
    mobile: bool,
    web: bool,
    auth_oidc: bool,
    quality_strict: bool,
    port_offset: u32,
    postgres_host_port: u16,
    api_host_port: u16,
    ops_host_port: u16,
    keycloak_host_port: u16,
    fake_provider_host_port: u16,
}

#[derive(Clone, Copy, Debug)]
struct PortConfiguration {
    postgres: u16,
    api: u16,
    ops: u16,
    keycloak: u16,
    fake_provider: u16,
}

impl PortConfiguration {
    fn new(offset: u32) -> Result<Self> {
        let shifted = [
            ("PostgreSQL", POSTGRES_HOST_PORT),
            ("API", API_HOST_PORT),
            ("operations", OPS_HOST_PORT),
            ("Keycloak", KEYCLOAK_HOST_PORT),
            ("fake provider", FAKE_PROVIDER_HOST_PORT),
        ]
        .map(|(name, base)| {
            base.checked_add(offset)
                .filter(|port| *port <= u16::MAX.into())
                .map(|port| (name, port as u16))
                .ok_or_else(|| {
                    anyhow!(
                        "--port-offset {offset} puts the {name} port above {}",
                        u16::MAX
                    )
                })
        });
        let [postgres, api, ops, keycloak, fake_provider] = shifted
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("fixed port list has five entries");
        let ports = [postgres.1, api.1, ops.1, keycloak.1, fake_provider.1];
        if ports.into_iter().collect::<BTreeSet<_>>().len() != ports.len() {
            bail!("--port-offset {offset} makes generated host ports collide");
        }
        Ok(Self {
            postgres: postgres.1,
            api: api.1,
            ops: ops.1,
            keycloak: keycloak.1,
            fake_provider: fake_provider.1,
        })
    }
}

pub fn generate_new(options: &NewOptions) -> Result<PathBuf> {
    validate_name(&options.name)?;
    let ports = PortConfiguration::new(options.port_offset)?;
    if options.worker && !options.backend {
        bail!("--worker requires --backend because it is generated in the backend workspace");
    }
    if !options.backend && !options.mobile && !options.web {
        bail!("select at least one capability: --backend, --mobile, or --web");
    }

    let destination = if options.into_existing {
        options.directory.clone()
    } else {
        options.directory.join(&options.name)
    };
    let non_empty = destination.exists()
        && fs::read_dir(&destination)
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

    let auth_oidc = options.auth == Some(AuthProvider::Oidc);
    let dependency = dependency_context(
        options.baukit_path.as_deref(),
        options.mobile,
        options.web,
        auth_oidc,
        options.worker,
    )?;
    let context = TemplateContext {
        app_name: options.name.clone(),
        app_crate: options.name.replace('-', "_"),
        app_env: options.name.replace('-', "_").to_ascii_uppercase(),
        template_version: TEMPLATE_VERSION.to_owned(),
        baukit_dependencies: dependency.cargo,
        baukit_web_typescript_dependencies: dependency.web_typescript,
        baukit_mobile_typescript_dependencies: dependency.mobile_typescript,
        baukit_manifest: dependency.manifest,
        baukit_dependency_description: dependency.description,
        baukit_typescript_dependency_description: dependency.typescript_description,
        product_description: product_description(options),
        backend: options.backend,
        worker: options.worker,
        mobile: options.mobile,
        web: options.web,
        auth_oidc,
        quality_strict: options.quality == QualityProfile::Strict,
        port_offset: options.port_offset,
        postgres_host_port: ports.postgres,
        api_host_port: ports.api,
        ops_host_port: ports.ops,
        keycloak_host_port: ports.keycloak,
        fake_provider_host_port: ports.fake_provider,
    };
    let rendered = render_product(&context, options)?;
    let mut conflicts = Vec::new();

    fs::create_dir_all(&destination)
        .with_context(|| format!("could not create {}", destination.display()))?;
    for (relative, bytes) in rendered {
        let output = destination.join(relative);
        if output.exists() {
            let existing = fs::read(&output)
                .with_context(|| format!("could not read existing file {}", output.display()))?;
            if existing != bytes {
                conflicts.push(output.strip_prefix(&destination)?.to_path_buf());
            }
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, bytes)
            .with_context(|| format!("could not write {}", output.display()))?;
    }

    if !conflicts.is_empty() {
        conflicts.sort();
        let mut report = String::from(
            "baukit generation conflicts\n\nThe following existing files differ from the template and were not overwritten:\n",
        );
        for conflict in &conflicts {
            report.push_str("- ");
            report.push_str(&conflict.display().to_string());
            report.push('\n');
        }
        let conflict_path = available_conflict_path(&destination)?;
        fs::write(&conflict_path, report)?;
        bail!(
            "generation found {} conflict(s); no modified file was overwritten. See {}",
            conflicts.len(),
            conflict_path.display()
        );
    }

    if options.resolve_lockfiles {
        resolve_lockfiles(&destination, options)?;
    }

    Ok(destination)
}

fn resolve_lockfiles(destination: &Path, options: &NewOptions) -> Result<()> {
    if options.backend && !destination.join("backend/Cargo.lock").is_file() {
        run_checked(
            Command::new("cargo").current_dir(destination).args([
                "generate-lockfile",
                "--manifest-path",
                "backend/Cargo.toml",
            ]),
            "Cargo lockfile generation",
        )?;
    }

    for capability in [(options.web, "web"), (options.mobile, "mobile")]
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
                    "pnpm@11.18.0",
                    "install",
                    "--lockfile-only",
                    "--ignore-scripts",
                ]),
            &format!("{capability} pnpm lockfile generation"),
        )?;
    }
    Ok(())
}

fn available_conflict_path(destination: &Path) -> Result<PathBuf> {
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

struct DependencyContext {
    cargo: String,
    web_typescript: String,
    mobile_typescript: String,
    manifest: String,
    description: String,
    typescript_description: String,
}

fn dependency_context(
    path: Option<&Path>,
    mobile: bool,
    web: bool,
    auth_oidc: bool,
    worker: bool,
) -> Result<DependencyContext> {
    let web_packages = typescript_packages(false, false, web && auth_oidc);
    let mobile_packages = typescript_packages(mobile, mobile && auth_oidc, false);
    if let Some(path) = path {
        let path = path.canonicalize().with_context(|| {
            format!(
                "--baukit-path {} does not resolve; point it at the repository's rust directory",
                path.display()
            )
        })?;
        let crates = path.join("crates");
        if !crates.join("baukit-config/Cargo.toml").is_file() {
            bail!(
                "--baukit-path {} is not a Baukit Rust workspace (missing crates/baukit-config/Cargo.toml)",
                path.display()
            );
        }
        let display = path.display().to_string().replace('\\', "\\\\");
        let mut names = vec![
            "baukit-config",
            "baukit-http",
            "baukit-openapi",
            "baukit-ops",
            "baukit-runtime",
            "baukit-telemetry",
            "baukit-test",
        ];
        if auth_oidc {
            names.push("baukit-auth");
        }
        if worker {
            names.push("baukit-jobs");
        }
        let cargo = names
            .iter()
            .map(|name| format!("{name} = {{ path = \"{display}/crates/{name}\" }}"))
            .collect::<Vec<_>>()
            .join("\n");
        let repository = path
            .parent()
            .ok_or_else(|| anyhow!("Baukit Rust workspace has no repository parent"))?;
        let typescript_root = repository.join("typescript");
        if mobile || web {
            for package in web_packages.iter().chain(&mobile_packages) {
                let directory = package.trim_start_matches("@baukit/");
                if !typescript_root
                    .join("packages")
                    .join(directory)
                    .join("package.json")
                    .is_file()
                {
                    bail!(
                        "--baukit-path {} has no matching TypeScript package `{package}` at {}",
                        path.display(),
                        typescript_root.display()
                    );
                }
            }
        }
        let typescript_display = typescript_root
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let render_typescript = |packages: &[&str]| {
            packages
                .iter()
                .map(|package| {
                    let directory = package.trim_start_matches("@baukit/");
                    format!("    \"{package}\": \"file:{typescript_display}/packages/{directory}\"")
                })
                .collect::<Vec<_>>()
                .join(",\n")
        };
        Ok(DependencyContext {
            cargo,
            web_typescript: render_typescript(&web_packages),
            mobile_typescript: render_typescript(&mobile_packages),
            manifest: format!("source = \"path\"\npath = \"{display}\""),
            description: format!("local path `{}`", path.display()),
            typescript_description: format!("local path `{}`", typescript_root.display()),
        })
    } else {
        let version = TEMPLATE_VERSION;
        let mut names = vec![
            "baukit-config",
            "baukit-http",
            "baukit-openapi",
            "baukit-ops",
            "baukit-runtime",
            "baukit-telemetry",
            "baukit-test",
        ];
        if auth_oidc {
            names.push("baukit-auth");
        }
        if worker {
            names.push("baukit-jobs");
        }
        let cargo = names
            .iter()
            .map(|name| format!("{name} = \"{version}\""))
            .collect::<Vec<_>>()
            .join("\n");
        let render_typescript = |packages: &[&str]| {
            packages
                .iter()
                .map(|package| format!("    \"{package}\": \"{version}\""))
                .collect::<Vec<_>>()
                .join(",\n")
        };
        Ok(DependencyContext {
            cargo,
            web_typescript: render_typescript(&web_packages),
            mobile_typescript: render_typescript(&mobile_packages),
            manifest: format!("source = \"registry\"\nversion = \"{version}\""),
            description: format!("crates.io version `{version}`"),
            typescript_description: format!("npm version `{version}`"),
        })
    }
}

fn typescript_packages(mobile: bool, mobile_auth: bool, web_auth: bool) -> Vec<&'static str> {
    let mut packages = EXPECTED_TYPESCRIPT_DEPENDENCIES.to_vec();
    if mobile {
        packages.extend(EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES);
    }
    if mobile_auth {
        packages.extend(EXPECTED_MOBILE_AUTH_DEPENDENCIES);
    }
    if web_auth {
        packages.extend(EXPECTED_WEB_AUTH_DEPENDENCIES);
    }
    packages
}

fn render_product(
    context: &TemplateContext,
    options: &NewOptions,
) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut environment = Environment::new();
    environment.set_keep_trailing_newline(true);
    let mut rendered = BTreeMap::new();
    render_directory(
        &COMMON_TEMPLATE,
        &environment,
        context,
        &mut rendered,
        false,
    )?;
    if let Some(guide) = rendered.get(Path::new("CLAUDE.md")).cloned() {
        rendered.insert(PathBuf::from("AGENTS.md"), guide);
    }
    if options.backend {
        render_directory(
            &BACKEND_TEMPLATE,
            &environment,
            context,
            &mut rendered,
            false,
        )?;
        if context.auth_oidc
            && let Some(overlay) = BACKEND_TEMPLATE.get_dir("__auth__")
        {
            render_directory(overlay, &environment, context, &mut rendered, true)?;
        }
        if options.worker {
            render_directory(
                &WORKER_TEMPLATE,
                &environment,
                context,
                &mut rendered,
                false,
            )?;
        }
    }
    if options.mobile {
        render_directory(
            &MOBILE_TEMPLATE,
            &environment,
            context,
            &mut rendered,
            false,
        )?;
        if context.auth_oidc
            && let Some(overlay) = MOBILE_TEMPLATE.get_dir("__auth__")
        {
            render_directory(overlay, &environment, context, &mut rendered, true)?;
        }
    }
    if options.web {
        render_directory(&WEB_TEMPLATE, &environment, context, &mut rendered, false)?;
        if context.auth_oidc
            && let Some(overlay) = WEB_TEMPLATE.get_dir("__auth__")
        {
            render_directory(overlay, &environment, context, &mut rendered, true)?;
        }
    }
    rendered.insert(
        PathBuf::from("baukit.toml"),
        render_manifest(context, options).into_bytes(),
    );
    Ok(rendered)
}

fn render_manifest(context: &TemplateContext, options: &NewOptions) -> String {
    let auth = match options.auth {
        Some(AuthProvider::Oidc) => "auth = \"oidc\"\n",
        None => "",
    };
    let port_offset = if options.port_offset == 0 {
        String::new()
    } else {
        format!("port_offset = {}\n", options.port_offset)
    };
    format!(
        "schema_version = {MANIFEST_SCHEMA_VERSION}\n\
template_version = \"{}\"\n\
{}\
\n\
[app]\n\
name = \"{}\"\n\
\n\
[quality]\n\
profile = \"{}\"\n\
backend_coverage_lines = {}\n\
critical_paths = []\n\
webkit_repeats = {}\n\
full_stack_e2e = false\n\
\n\
[capabilities]\n\
backend = {}\n\
worker = {}\n\
mobile = {}\n\
web = {}\n\
pwa = false\n\
{}\
\n\
[dependencies.baukit]\n\
{}\n\
\n\
[openapi]\n\
schema = \"backend/openapi.json\"\n\
consumers = [\"generated/openapi.d.ts\"]\n",
        context.template_version,
        port_offset,
        context.app_name,
        match options.quality {
            QualityProfile::Standard => "standard",
            QualityProfile::Strict => "strict",
        },
        default_backend_coverage_lines(),
        default_webkit_repeats(),
        options.backend,
        options.worker,
        options.mobile,
        options.web,
        auth,
        context.baukit_manifest,
    )
}

fn render_directory(
    directory: &Dir<'_>,
    environment: &Environment<'_>,
    context: &TemplateContext,
    rendered: &mut BTreeMap<PathBuf, Vec<u8>>,
    auth_overlay: bool,
) -> Result<()> {
    for file in directory.files() {
        let relative = file.path();
        if is_auth_only(relative) && (!context.auth_oidc || !auth_overlay) {
            continue;
        }
        if is_strict_only(relative) && !context.quality_strict {
            continue;
        }
        if is_backend_only(relative) && !context.backend {
            continue;
        }
        let source = file
            .contents_utf8()
            .ok_or_else(|| anyhow!("template {} is not UTF-8", relative.display()))?;
        let mut name = relative
            .to_string_lossy()
            .replace("__auth__/", "")
            .replace("__strict__/", "")
            .replace("__backend__/", "");
        if name.ends_with(".jinja") {
            name.truncate(name.len() - ".jinja".len());
        }
        let mut output = environment.render_str(source, context!(context))?;
        if name.ends_with("Cargo.toml") && context.app_crate != context.app_name {
            output = output.replace(
                &format!("{}-", context.app_crate),
                &format!("{}-", context.app_name),
            );
        }
        let output_path = PathBuf::from(name.replace("__app__", &context.app_name));
        rendered.insert(output_path, output.into_bytes());
    }
    for child in directory.dirs() {
        if is_auth_only(child.path()) && !auth_overlay {
            continue;
        }
        if is_strict_only(child.path()) && !context.quality_strict {
            continue;
        }
        if is_backend_only(child.path()) && !context.backend {
            continue;
        }
        render_directory(child, environment, context, rendered, auth_overlay)?;
    }
    Ok(())
}

fn product_description(options: &NewOptions) -> String {
    let mut capabilities = Vec::new();
    if options.backend {
        capabilities.push("Rust backend");
    }
    if options.worker {
        capabilities.push("durable worker");
    }
    if options.web {
        capabilities.push("web app");
    }
    if options.mobile {
        capabilities.push("mobile app");
    }
    format!("Baukit product with {}", capabilities.join(", "))
}

fn is_auth_only(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__auth__")
}

fn is_strict_only(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__strict__")
}

fn is_backend_only(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__backend__")
}

pub fn read_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join("baukit.toml");
    let source = fs::read_to_string(&path).with_context(|| {
        format!(
            "could not read {}; run this command at a generated product root",
            path.display()
        )
    })?;
    toml::from_str(&source).with_context(|| format!("could not parse {}", path.display()))
}

pub fn doctor(root: &Path) -> Result<Vec<String>> {
    doctor_with_host(root, &SystemDoctorHost)
}

#[derive(Debug)]
struct DoctorCommandOutput {
    success: bool,
    code: Option<i32>,
    stderr: String,
}

trait DoctorHost {
    fn env_var_os(&self, name: &str) -> Option<OsString>;
    fn is_socket(&self, path: &Path) -> bool;
    fn run_command(
        &self,
        program: &str,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> io::Result<DoctorCommandOutput>;
}

struct SystemDoctorHost;

impl DoctorHost for SystemDoctorHost {
    fn env_var_os(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn is_socket(&self, path: &Path) -> bool {
        is_socket(path)
    }

    fn run_command(
        &self,
        program: &str,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> io::Result<DoctorCommandOutput> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(directory) = current_dir {
            command.current_dir(directory);
        }
        if program == "git" {
            command.env("GIT_TERMINAL_PROMPT", "0").env(
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o IdentityFile=none",
            );
        }
        let output = command.output()?;
        Ok(DoctorCommandOutput {
            success: output.status.success(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

#[cfg(unix)]
fn is_socket(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(not(unix))]
fn is_socket(_path: &Path) -> bool {
    false
}

fn doctor_with_host(root: &Path, host: &dyn DoctorHost) -> Result<Vec<String>> {
    let manifest = read_manifest(root)?;
    let mut failures = Vec::new();
    let mut successes = Vec::new();
    for relative in EXPECTED_COMMON_FILES {
        if !root.join(relative).is_file() {
            failures.push(format!("missing expected product file `{relative}`"));
        }
    }
    if manifest.quality.profile == QualityProfile::Strict {
        for relative in EXPECTED_STRICT_FILES {
            if !root.join(relative).is_file() {
                failures.push(format!("missing expected strict quality file `{relative}`"));
            }
        }
        if manifest.capabilities.backend {
            for relative in EXPECTED_STRICT_BACKEND_FILES {
                if !root.join(relative).is_file() {
                    failures.push(format!("missing expected strict backend file `{relative}`"));
                }
            }
        }
        if manifest.quality.full_stack_e2e && !root.join("scripts/full-stack-e2e.sh").is_file() {
            failures.push("quality.full_stack_e2e requires `scripts/full-stack-e2e.sh`".to_owned());
        }
    }
    if !(1..=100).contains(&manifest.quality.backend_coverage_lines) {
        failures.push("quality.backend_coverage_lines must be between 1 and 100".to_owned());
    }
    if manifest.quality.webkit_repeats == 0 {
        failures.push("quality.webkit_repeats must be greater than zero".to_owned());
    }
    if !manifest.capabilities.web && !manifest.quality.critical_paths.is_empty() {
        failures.push("quality.critical_paths requires the web capability".to_owned());
    }
    for critical_path in &manifest.quality.critical_paths {
        let path = Path::new(critical_path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
            || !critical_path.starts_with("e2e/tests/")
            || !critical_path.ends_with(".spec.ts")
        {
            failures.push(format!(
                "critical path `{critical_path}` must be an e2e/tests/*.spec.ts file"
            ));
        } else if manifest.capabilities.web && !root.join("web").join(path).is_file() {
            failures.push(format!(
                "critical path `web/{critical_path}` does not exist"
            ));
        }
    }
    if manifest.capabilities.pwa && !manifest.capabilities.web {
        failures.push("the PWA capability requires the web capability".to_owned());
    }
    if manifest.schema_version == MANIFEST_SCHEMA_VERSION {
        successes.push(format!(
            "manifest schema {} is current",
            MANIFEST_SCHEMA_VERSION
        ));
    } else {
        failures.push(format!(
            "baukit.toml schema_version is {}, but this CLI supports {}; regenerate or upgrade the manifest",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    if manifest.template_version == TEMPLATE_VERSION {
        successes.push(format!(
            "template version {} matches the CLI",
            TEMPLATE_VERSION
        ));
    } else {
        failures.push(format!(
            "product template version {} differs from CLI version {}; run the matching CLI or upgrade the product",
            manifest.template_version, TEMPLATE_VERSION
        ));
    }
    validate_port_configuration(root, &manifest, &mut successes, &mut failures)?;
    if manifest.capabilities.backend {
        validate_openapi_paths(root, &manifest.openapi, &mut successes, &mut failures);
        for expected in EXPECTED_BACKEND_FILES {
            let relative = expected.replace("__APP__", &manifest.app.name);
            if !root.join(&relative).is_file() {
                failures.push(format!("missing expected backend file `{relative}`"));
            }
        }
        validate_migrations(root, &mut successes, &mut failures)?;
        if manifest.capabilities.worker {
            validate_jobs_migration(root, &mut successes, &mut failures)?;
            for expected in EXPECTED_WORKER_FILES {
                let relative = expected.replace("__APP__", &manifest.app.name);
                if !root.join(&relative).is_file() {
                    failures.push(format!("missing expected worker file `{relative}`"));
                }
            }
        }
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            for relative in EXPECTED_AUTH_BACKEND_FILES {
                if !root.join(relative).is_file() {
                    failures.push(format!("missing expected OIDC backend file `{relative}`"));
                }
            }
        }
        let cargo = root.join("backend/Cargo.toml");
        if cargo.is_file() {
            let args = vec![
                "metadata".to_owned(),
                "--manifest-path".to_owned(),
                cargo.to_string_lossy().into_owned(),
                "--format-version".to_owned(),
                "1".to_owned(),
                "--no-deps".to_owned(),
            ];
            let output = host
                .run_command("cargo", &args, None)
                .context("could not run cargo metadata; install Rust from https://rustup.rs")?;
            if output.success {
                successes.push("Cargo workspace and dependency manifests parse".to_owned());
            } else {
                failures.push(format!("Cargo workspace does not parse: {}", output.stderr));
            }
        }
    }
    if manifest.capabilities.mobile {
        let mut dependencies = vec![
            "expo",
            "expo-constants",
            "expo-linking",
            "expo-localization",
            "expo-router",
            "expo-sqlite",
            "i18next",
            "react-i18next",
            "react-native",
            "react-native-gesture-handler",
            "react-native-reanimated",
            "react-native-safe-area-context",
            "react-native-screens",
            "react-native-worklets",
        ];
        dependencies.extend(EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES);
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            dependencies.extend(EXPECTED_MOBILE_AUTH_DEPENDENCIES);
        }
        validate_frontend_capability(
            root,
            "mobile",
            EXPECTED_MOBILE_FILES,
            &dependencies,
            &mut successes,
            &mut failures,
        )?;
        validate_mobile_router_configuration(root, &mut successes, &mut failures)?;
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            for relative in EXPECTED_AUTH_MOBILE_FILES {
                if !root.join(relative).is_file() {
                    failures.push(format!("missing expected OIDC mobile file `{relative}`"));
                }
            }
        }
    }
    if manifest.capabilities.web {
        let mut dependencies = vec!["@tanstack/react-query", "vite"];
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            dependencies.extend(EXPECTED_WEB_AUTH_DEPENDENCIES);
        }
        validate_frontend_capability(
            root,
            "web",
            EXPECTED_WEB_FILES,
            &dependencies,
            &mut successes,
            &mut failures,
        )?;
        if manifest.quality.profile == QualityProfile::Strict && manifest.capabilities.pwa {
            let package_json = fs::read_to_string(root.join("web/package.json"))?;
            if !package_json.contains("\"build:sw:check\"") {
                failures.push(
                    "the strict PWA capability requires the web `build:sw:check` script".to_owned(),
                );
            }
        }
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            for relative in EXPECTED_AUTH_WEB_FILES {
                if !root.join(relative).is_file() {
                    failures.push(format!("missing expected OIDC web file `{relative}`"));
                }
            }
        }
    }
    let ssh_agent_ready = match &manifest.dependencies.baukit {
        BaukitDependency::Path { path } => {
            if Path::new(path)
                .join("crates/baukit-config/Cargo.toml")
                .is_file()
            {
                successes.push(format!("Baukit path dependency resolves at {path}"));
            } else {
                failures.push(format!(
                    "Baukit path dependency `{path}` is unavailable; regenerate with --baukit-path or restore that checkout"
                ));
            }
            None
        }
        BaukitDependency::Git { git, tag } => {
            let ready = diagnose_ssh_agent(host, &mut successes, &mut failures);
            probe_git_dependency(host, git, tag, &mut successes, &mut failures);
            Some(ready)
        }
        BaukitDependency::Registry { version } => {
            successes.push(format!(
                "Baukit dependencies resolve from crates.io and npm at version {version}"
            ));
            None
        }
    };
    if has_docker_build_targets(root) {
        let docker_ready = diagnose_docker(host, &mut successes, &mut failures);
        match ssh_agent_ready {
            Some(true) if docker_ready => successes
                .push("Docker BuildKit SSH forwarding prerequisites are present".to_owned()),
            Some(false) if docker_ready => failures.push(
                "Docker and BuildKit are available, but SSH forwarding is not ready; fix the SSH agent problem above"
                    .to_owned(),
            ),
            None if docker_ready => successes.push(
                "Docker image builds are ready; registry and local path dependencies do not require SSH forwarding"
                    .to_owned(),
            ),
            Some(_) | None => {}
        }
    }
    if failures.is_empty() {
        Ok(successes)
    } else {
        bail!(
            "doctor found {} problem(s):\n- {}",
            failures.len(),
            failures.join("\n- ")
        )
    }
}

fn validate_port_configuration(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let ports = match PortConfiguration::new(manifest.port_offset) {
        Ok(ports) => ports,
        Err(error) => {
            failures.push(format!("invalid port_offset: {error}"));
            return Ok(());
        }
    };
    let initial_failure_count = failures.len();
    let mut expected = Vec::new();
    if manifest.capabilities.backend {
        expected.extend([
            ("compose.yaml", format!("\"{}:5432\"", ports.postgres)),
            ("README.md", format!("port {}", ports.api)),
            ("README.md", format!("port {}", ports.ops)),
            (
                "docs/fake-providers.md",
                format!("FAKE_PROVIDER_PORT:-{}", ports.fake_provider),
            ),
        ]);
        if manifest.port_offset > 0 {
            expected.extend([
                ("Makefile", format!("HTTP__PORT={}", ports.api)),
                ("Makefile", format!("OPS__PORT={}", ports.ops)),
                ("deploy/values.yaml", format!("http: {}", ports.api)),
                ("deploy/values.yaml", format!("ops: {}", ports.ops)),
            ]);
        }
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            expected.extend([
                ("compose.yaml", format!("\"{}:8080\"", ports.keycloak)),
                (
                    "scripts/pkce-login.py",
                    format!("localhost:{}/me", ports.api),
                ),
                (
                    "backend/crates/PLACEHOLDER-bin/src/lib.rs",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
            ]);
        }
    }
    if manifest.capabilities.mobile {
        expected.extend([
            ("mobile/.env.example", format!("localhost:{}", ports.api)),
            ("mobile/app.config.ts", format!("localhost:{}", ports.api)),
            ("mobile/src/api.ts", format!("localhost:{}", ports.api)),
        ]);
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            expected.extend([
                (
                    "mobile/.env.example",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "mobile/app.config.ts",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "mobile/src/auth.ts",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
            ]);
        }
    }
    if manifest.capabilities.web {
        expected.extend([
            ("web/.env.example", format!("localhost:{}", ports.api)),
            ("web/src/api.ts", format!("localhost:{}", ports.api)),
        ]);
        if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
            expected.extend([
                (
                    "web/.env.example",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "web/src/auth.ts",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
            ]);
        }
    }
    for (relative, snippet) in expected {
        let relative = relative.replace("PLACEHOLDER", &manifest.app.name);
        let path = root.join(&relative);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let mismatched = if is_environment_capable_source(&relative) {
            let expected_port = localhost_port(&snippet).expect("expected snippet has a port");
            has_mismatched_localhost_port(&source, expected_port)
        } else {
            !source.contains(&snippet)
        };
        if mismatched {
            failures.push(format!(
                "generated file `{relative}` does not use port offset {}",
                manifest.port_offset
            ));
        }
    }
    if failures.len() == initial_failure_count {
        successes.push(format!(
            "generated files use port offset {}",
            manifest.port_offset
        ));
    }
    Ok(())
}

fn is_environment_capable_source(relative: &str) -> bool {
    matches!(
        relative,
        "mobile/src/api.ts" | "mobile/src/auth.ts" | "web/src/api.ts" | "web/src/auth.ts"
    )
}

fn localhost_port(snippet: &str) -> Option<u16> {
    snippet
        .split_once("localhost:")?
        .1
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn has_mismatched_localhost_port(source: &str, expected_port: u16) -> bool {
    let mut found_numeric_port = false;
    for (index, marker) in source.match_indices("localhost:") {
        let port = source[index + marker.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if let Ok(port) = port.parse::<u16>() {
            found_numeric_port = true;
            if port == expected_port {
                return false;
            }
        }
    }
    found_numeric_port
}

fn diagnose_ssh_agent(
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> bool {
    let Some(socket) = host
        .env_var_os("SSH_AUTH_SOCK")
        .filter(|value| !value.is_empty())
    else {
        failures.push(
            "SSH_AUTH_SOCK is unset; start an agent with `eval \"$(ssh-agent -s)\"`, then load a key with `ssh-add ~/.ssh/<private-key>`"
                .to_owned(),
        );
        return false;
    };
    if !host.is_socket(Path::new(&socket)) {
        failures.push(
            "SSH_AUTH_SOCK does not point to a socket; restart the agent, then load a key with `ssh-add ~/.ssh/<private-key>`"
                .to_owned(),
        );
        return false;
    }

    let args = vec!["-l".to_owned()];
    match host.run_command("ssh-add", &args, None) {
        Ok(output) if output.success => {
            successes.push("SSH agent is usable and has at least one loaded identity".to_owned());
            true
        }
        Ok(output) if output.code == Some(1) => {
            failures.push(
                "SSH agent has no loaded identities; add one with `ssh-add ~/.ssh/<private-key>`"
                    .to_owned(),
            );
            false
        }
        Ok(output) if output.code == Some(2) => {
            failures.push(
                "SSH agent is unusable; restart it with `eval \"$(ssh-agent -s)\"`, then add a key with `ssh-add ~/.ssh/<private-key>`"
                    .to_owned(),
            );
            false
        }
        Ok(_) => {
            failures.push(
                "could not query the SSH agent; restart it, then add a key with `ssh-add ~/.ssh/<private-key>`"
                    .to_owned(),
            );
            false
        }
        Err(_) => {
            failures.push(
                "could not run `ssh-add`; install an OpenSSH client, start an agent, and load a key"
                    .to_owned(),
            );
            false
        }
    }
}

fn probe_git_dependency(
    host: &dyn DoctorHost,
    git: &str,
    tag: &str,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    let args = ["ls-remote", "--exit-code", "--tags", git, tag]
        .map(str::to_owned)
        .to_vec();
    match host.run_command("git", &args, None) {
        Ok(output) if output.success => successes.push(format!(
            "Baukit git dependency resolves at tag {tag} through the current SSH identity"
        )),
        Ok(_) => failures.push(format!(
            "Baukit git dependency `{git}` tag `{tag}` is not resolvable; check network access, SSH repository access, and the release tag"
        )),
        Err(error) => failures.push(format!(
            "could not run git to resolve Baukit dependency: {error}"
        )),
    }
}

fn has_docker_build_targets(root: &Path) -> bool {
    ["compose.yaml", "Dockerfile", "backend/Dockerfile"]
        .iter()
        .any(|relative| root.join(relative).is_file())
}

fn diagnose_docker(
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> bool {
    let version_args = ["version", "--format", "{{.Server.Version}}"]
        .map(str::to_owned)
        .to_vec();
    match host.run_command("docker", &version_args, None) {
        Ok(output) if output.success => {}
        Ok(_) => {
            failures.push(
                "Docker daemon is unavailable; start Docker before building generated images"
                    .to_owned(),
            );
            return false;
        }
        Err(_) => {
            failures.push(
                "Docker is unavailable; install Docker and start it before building generated images"
                    .to_owned(),
            );
            return false;
        }
    }

    let buildx_args = vec!["buildx".to_owned(), "version".to_owned()];
    match host.run_command("docker", &buildx_args, None) {
        Ok(output) if output.success => {
            successes.push("Docker daemon and BuildKit are available".to_owned());
            true
        }
        Ok(_) | Err(_) => {
            failures.push(
                "Docker BuildKit is unavailable; install or enable Docker Buildx before building generated images"
                    .to_owned(),
            );
            false
        }
    }
}

fn validate_openapi_paths(
    root: &Path,
    openapi: &OpenApiPaths,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    let initial_failure_count = failures.len();
    validate_openapi_file(root, "schema", &openapi.schema, failures);
    let consumers = openapi.consumers();
    if consumers.is_empty() {
        failures.push("openapi.consumers must list at least one output".to_owned());
    }
    for consumer in consumers {
        validate_openapi_file(root, "consumer", consumer, failures);
    }
    if failures.len() == initial_failure_count {
        successes.push("manifest-declared OpenAPI files are present".to_owned());
    }
}

fn validate_openapi_file(root: &Path, kind: &str, relative: &str, failures: &mut Vec<String>) {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        failures.push(format!(
            "OpenAPI {kind} `{relative}` must stay inside the product root"
        ));
    } else if !root.join(path).is_file() {
        failures.push(format!("missing OpenAPI {kind} file `{relative}`"));
    }
}

fn validate_migrations(
    root: &Path,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let directory = root.join("backend/migrations");
    if !directory.is_dir() {
        failures.push("missing backend migration directory `backend/migrations`".to_owned());
        return Ok(());
    }
    let mut migrations = Vec::new();
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("could not inspect {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            migrations.push(path);
        }
    }
    migrations.sort();
    if migrations.is_empty() {
        failures.push("backend migration directory contains no `.sql` migrations".to_owned());
    } else {
        successes.push(format!(
            "backend migration directory contains {} SQL migration(s)",
            migrations.len()
        ));
    }
    Ok(())
}

fn validate_jobs_migration(
    root: &Path,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let directory = root.join("backend/migrations");
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("could not inspect {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            if sql_creates_table(&source, "job_outbox") {
                successes.push(
                    "a backend migration creates the baukit-jobs `job_outbox` table".to_owned(),
                );
                return Ok(());
            }
        }
    }
    failures.push("no backend migration creates the baukit-jobs `job_outbox` table".to_owned());
    Ok(())
}

fn sql_creates_table(source: &str, table: &str) -> bool {
    let uncommented = source
        .lines()
        .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
        .collect::<Vec<_>>()
        .join(" ");
    let tokens = uncommented
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();

    tokens.iter().enumerate().any(|(index, token)| {
        if token != "create" || tokens.get(index + 1).is_none_or(|token| token != "table") {
            return false;
        }
        let mut name_index = index + 2;
        if tokens.get(name_index).is_some_and(|token| token == "if")
            && tokens
                .get(name_index + 1)
                .is_some_and(|token| token == "not")
            && tokens
                .get(name_index + 2)
                .is_some_and(|token| token == "exists")
        {
            name_index += 3;
        }
        tokens.get(name_index).is_some_and(|token| token == table)
            || tokens
                .get(name_index + 1)
                .is_some_and(|token| token == table)
    })
}

fn validate_frontend_capability(
    root: &Path,
    capability: &str,
    expected_files: &[&str],
    target_dependencies: &[&str],
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    for relative in expected_files {
        if !root.join(relative).is_file() {
            failures.push(format!("missing expected {capability} file `{relative}`"));
        }
    }

    let package_path = root.join(capability).join("package.json");
    if package_path.is_file() {
        let package = fs::read_to_string(&package_path)
            .with_context(|| format!("could not read {}", package_path.display()))?;
        for dependency in EXPECTED_TYPESCRIPT_DEPENDENCIES
            .iter()
            .chain(target_dependencies)
        {
            if !package.contains(&format!("\"{dependency}\"")) {
                failures.push(format!(
                    "{capability}/package.json is missing dependency `{dependency}`"
                ));
            }
        }
        successes.push(format!(
            "{capability} template files and Baukit dependencies are present"
        ));
    }
    Ok(())
}

fn validate_mobile_router_configuration(
    root: &Path,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let package_path = root.join("mobile/package.json");
    if package_path.is_file() {
        let package = fs::read_to_string(&package_path)
            .with_context(|| format!("could not read {}", package_path.display()))?;
        if !package.contains("\"main\": \"expo-router/entry\"") {
            failures.push("mobile/package.json must set `main` to `expo-router/entry`".to_owned());
        }
    }

    let config_path = root.join("mobile/app.config.ts");
    if config_path.is_file() {
        let config = fs::read_to_string(&config_path)
            .with_context(|| format!("could not read {}", config_path.display()))?;
        if !config.contains("scheme:") {
            failures.push("mobile/app.config.ts must declare a deep-link scheme".to_owned());
        }
    }

    if failures.iter().all(|failure| {
        !failure.contains("expo-router/entry") && !failure.contains("deep-link scheme")
    }) {
        successes
            .push("mobile Expo Router entry point and deep-link scheme are configured".to_owned());
    }
    Ok(())
}

pub fn generate_openapi_client(root: &Path) -> Result<()> {
    let manifest = read_manifest(root)?;
    if !manifest.capabilities.backend {
        bail!("this product has no backend capability");
    }
    let consumers = manifest.openapi.consumers();
    if consumers.is_empty() {
        bail!("baukit.toml lists no OpenAPI consumers");
    }
    let corepack = command_exists("corepack");
    let pnpm = command_exists("pnpm");
    let npx = command_exists("npx");
    if !corepack && !pnpm && !npx {
        bail!(
            "TypeScript generation needs current Node.js LTS with corepack, pnpm, or npx; the committed OpenAPI schema was left unchanged"
        );
    }
    for consumer in consumers {
        let output = root.join(consumer);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut command = if corepack {
            let mut command = Command::new("corepack");
            command.args(["pnpm", "dlx", OPENAPI_TYPESCRIPT_PACKAGE]);
            command
        } else if pnpm {
            let mut command = Command::new("pnpm");
            command.args(["dlx", OPENAPI_TYPESCRIPT_PACKAGE]);
            command
        } else {
            let mut command = Command::new("npx");
            command.args(["--yes", OPENAPI_TYPESCRIPT_PACKAGE]);
            command
        };
        command
            .current_dir(root)
            .args([&manifest.openapi.schema, "-o", consumer]);
        run_checked(
            &mut command,
            &format!("TypeScript client generation for {consumer}"),
        )?;
    }
    Ok(())
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_checked(command: &mut Command, label: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not start {label}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{label} failed with {status}")
    }
}

fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.contains("--");
    if valid {
        Ok(())
    } else {
        bail!(
            "invalid application name `{name}`; use 1-64 lowercase ASCII letters, digits, and single hyphens, starting with a letter"
        )
    }
}

#[cfg(test)]
mod doctor_tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        ffi::OsString,
        fs, io,
        path::{Path, PathBuf},
    };

    use super::{
        DoctorCommandOutput, DoctorHost, diagnose_docker, diagnose_ssh_agent,
        has_mismatched_localhost_port, probe_git_dependency, validate_mobile_router_configuration,
    };

    struct ExpectedCommand {
        program: &'static str,
        args: Vec<&'static str>,
        result: io::Result<DoctorCommandOutput>,
    }

    struct FakeHost {
        ssh_auth_sock: Option<OsString>,
        socket: bool,
        commands: RefCell<VecDeque<ExpectedCommand>>,
    }

    impl FakeHost {
        fn new(ssh_auth_sock: Option<&str>, socket: bool, commands: Vec<ExpectedCommand>) -> Self {
            Self {
                ssh_auth_sock: ssh_auth_sock.map(OsString::from),
                socket,
                commands: RefCell::new(commands.into()),
            }
        }

        fn assert_finished(&self) {
            assert!(self.commands.borrow().is_empty());
        }
    }

    impl DoctorHost for FakeHost {
        fn env_var_os(&self, name: &str) -> Option<OsString> {
            assert_eq!(name, "SSH_AUTH_SOCK");
            self.ssh_auth_sock.clone()
        }

        fn is_socket(&self, path: &Path) -> bool {
            assert_eq!(path, PathBuf::from("/agent.sock"));
            self.socket
        }

        fn run_command(
            &self,
            program: &str,
            args: &[String],
            current_dir: Option<&Path>,
        ) -> io::Result<DoctorCommandOutput> {
            assert!(current_dir.is_none());
            let expected = self
                .commands
                .borrow_mut()
                .pop_front()
                .expect("unexpected command");
            assert_eq!(program, expected.program);
            assert_eq!(
                args,
                &expected
                    .args
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            );
            expected.result
        }
    }

    fn output(code: i32) -> io::Result<DoctorCommandOutput> {
        Ok(DoctorCommandOutput {
            success: code == 0,
            code: Some(code),
            stderr: String::new(),
        })
    }

    #[test]
    fn mobile_router_configuration_requires_the_entry_point_and_scheme() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("mobile"))?;
        fs::write(
            root.path().join("mobile/package.json"),
            "{\"main\":\"expo/AppEntry.js\"}\n",
        )?;
        fs::write(
            root.path().join("mobile/app.config.ts"),
            "export default { slug: 'product' };\n",
        )?;
        let mut successes = Vec::new();
        let mut failures = Vec::new();

        validate_mobile_router_configuration(root.path(), &mut successes, &mut failures)?;

        assert!(successes.is_empty());
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("expo-router/entry"))
        );
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("deep-link scheme"))
        );
        Ok(())
    }

    #[test]
    fn localhost_check_allows_an_expected_service_port_with_a_frontend_origin() {
        let source = r#"
            const issuer = "http://localhost:8081/realms/product";
            const origin = "http://localhost:5173";
        "#;
        assert!(!has_mismatched_localhost_port(source, 8081));
        assert!(has_mismatched_localhost_port(source, 8181));
    }

    #[test]
    fn ssh_agent_reports_missing_and_non_socket_values_without_running_commands() {
        for (socket, is_socket, expected) in [
            (None, false, "SSH_AUTH_SOCK is unset"),
            (
                Some("/agent.sock"),
                false,
                "SSH_AUTH_SOCK does not point to a socket",
            ),
        ] {
            let host = FakeHost::new(socket, is_socket, Vec::new());
            let mut successes = Vec::new();
            let mut failures = Vec::new();
            assert!(!diagnose_ssh_agent(&host, &mut successes, &mut failures));
            assert!(successes.is_empty());
            assert_eq!(failures.len(), 1);
            assert!(failures[0].contains(expected));
            assert!(failures[0].contains("ssh-add"));
            assert!(!failures[0].contains("/agent.sock"));
            host.assert_finished();
        }
    }

    #[test]
    fn ssh_agent_distinguishes_no_identities_from_an_unusable_agent() {
        for (code, expected) in [(1, "no loaded identities"), (2, "agent is unusable")] {
            let host = FakeHost::new(
                Some("/agent.sock"),
                true,
                vec![ExpectedCommand {
                    program: "ssh-add",
                    args: vec!["-l"],
                    result: output(code),
                }],
            );
            let mut successes = Vec::new();
            let mut failures = Vec::new();
            assert!(!diagnose_ssh_agent(&host, &mut successes, &mut failures));
            assert!(failures[0].contains(expected));
            assert!(failures[0].contains("ssh-add"));
            host.assert_finished();
        }
    }

    #[test]
    fn ssh_agent_accepts_a_socket_with_a_loaded_identity() {
        let host = FakeHost::new(
            Some("/agent.sock"),
            true,
            vec![ExpectedCommand {
                program: "ssh-add",
                args: vec!["-l"],
                result: output(0),
            }],
        );
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        assert!(diagnose_ssh_agent(&host, &mut successes, &mut failures));
        assert!(failures.is_empty());
        assert!(successes[0].contains("loaded identity"));
        host.assert_finished();
    }

    #[test]
    fn git_probe_uses_only_the_remote_and_tag() {
        let host = FakeHost::new(
            None,
            false,
            vec![ExpectedCommand {
                program: "git",
                args: vec![
                    "ls-remote",
                    "--exit-code",
                    "--tags",
                    "ssh://git@example.test/baukit.git",
                    "v1.2.3",
                ],
                result: output(0),
            }],
        );
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        probe_git_dependency(
            &host,
            "ssh://git@example.test/baukit.git",
            "v1.2.3",
            &mut successes,
            &mut failures,
        );
        assert!(failures.is_empty());
        assert!(successes[0].contains("current SSH identity"));
        host.assert_finished();
    }

    #[test]
    fn docker_diagnostic_checks_the_daemon_then_buildkit() {
        let host = FakeHost::new(
            None,
            false,
            vec![
                ExpectedCommand {
                    program: "docker",
                    args: vec!["version", "--format", "{{.Server.Version}}"],
                    result: output(0),
                },
                ExpectedCommand {
                    program: "docker",
                    args: vec!["buildx", "version"],
                    result: output(0),
                },
            ],
        );
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        assert!(diagnose_docker(&host, &mut successes, &mut failures));
        assert!(failures.is_empty());
        assert!(successes[0].contains("BuildKit"));
        host.assert_finished();
    }

    #[test]
    fn docker_diagnostic_reports_a_missing_buildkit_plugin() {
        let host = FakeHost::new(
            None,
            false,
            vec![
                ExpectedCommand {
                    program: "docker",
                    args: vec!["version", "--format", "{{.Server.Version}}"],
                    result: output(0),
                },
                ExpectedCommand {
                    program: "docker",
                    args: vec!["buildx", "version"],
                    result: output(1),
                },
            ],
        );
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        assert!(!diagnose_docker(&host, &mut successes, &mut failures));
        assert!(successes.is_empty());
        assert!(failures[0].contains("BuildKit is unavailable"));
        host.assert_finished();
    }
}
