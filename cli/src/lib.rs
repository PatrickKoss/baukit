use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use include_dir::{Dir, include_dir};
use minijinja::{Environment, context};
use serde::{Deserialize, Serialize};

static BACKEND_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/backend");
static MOBILE_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/mobile");
static WEB_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/web");

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const TEMPLATE_VERSION: &str = include_str!("../../templates/VERSION").trim_ascii();

const EXPECTED_BACKEND_FILES: &[&str] = &[
    "README.md",
    "Makefile",
    ".github/workflows/ci.yml",
    "compose.yaml",
    "deploy/values.yaml",
    ".agents/skills/baukit-backend/SKILL.md",
    ".claude/skills/baukit-backend/SKILL.md",
    "scripts/openapi.sh",
    "backend/Cargo.toml",
    "backend/migrations/0001_create_items.sql",
    "backend/openapi.json",
    "backend/crates/__APP__-domain/Cargo.toml",
    "backend/crates/__APP__-ports/Cargo.toml",
    "backend/crates/__APP__-services/Cargo.toml",
    "backend/crates/__APP__-api/Cargo.toml",
    "backend/crates/__APP__-postgres/Cargo.toml",
    "backend/crates/__APP__-bin/Cargo.toml",
];

const EXPECTED_MOBILE_FILES: &[&str] = &[
    "mobile/package.json",
    "mobile/app.config.ts",
    "mobile/tsconfig.json",
    "mobile/eslint.config.js",
    "mobile/vitest.config.ts",
    "mobile/App.tsx",
    "mobile/scripts/generate-tokens.mjs",
    "mobile/src/api.ts",
    "mobile/src/analytics.ts",
    "mobile/src/theme.ts",
    "mobile/src/tokens.ts",
];

const EXPECTED_WEB_FILES: &[&str] = &[
    "web/package.json",
    "web/index.html",
    "web/vite.config.ts",
    "web/tsconfig.json",
    "web/eslint.config.js",
    "web/vitest.config.ts",
    "web/src/App.tsx",
    "web/src/api.ts",
    "web/src/analytics.ts",
    "web/src/tokens.css",
];

const EXPECTED_TYPESCRIPT_DEPENDENCIES: &[&str] = &[
    "@baukit/analytics-core",
    "@baukit/api-runtime",
    "@baukit/ui-tokens",
];

#[derive(Clone, Debug)]
pub struct NewOptions {
    pub name: String,
    pub directory: PathBuf,
    pub backend: bool,
    pub mobile: bool,
    pub web: bool,
    pub force: bool,
    pub baukit_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub template_version: String,
    pub app: AppManifest,
    pub capabilities: Capabilities,
    pub dependencies: Dependencies,
    pub openapi: OpenApiPaths,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppManifest {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Capabilities {
    pub backend: bool,
    pub mobile: bool,
    pub web: bool,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenApiPaths {
    pub schema: String,
    pub typescript: String,
}

#[derive(Debug, Serialize)]
struct TemplateContext {
    app_name: String,
    app_crate: String,
    app_env: String,
    template_version: String,
    baukit_dependencies: String,
    baukit_typescript_dependencies: String,
    baukit_manifest: String,
    baukit_dependency_description: String,
    baukit_typescript_dependency_description: String,
}

pub fn generate_new(options: &NewOptions) -> Result<PathBuf> {
    validate_name(&options.name)?;
    if !options.backend && !options.mobile && !options.web {
        bail!("select at least one capability: --backend, --mobile, or --web");
    }

    let destination = options.directory.join(&options.name);
    let non_empty = destination.exists()
        && fs::read_dir(&destination)
            .with_context(|| format!("could not inspect {}", destination.display()))?
            .next()
            .transpose()?
            .is_some();
    if non_empty && !options.force {
        bail!(
            "destination {} is not empty; choose an empty directory or pass --force to add only non-conflicting files",
            destination.display()
        );
    }

    let dependency = dependency_context(
        options.baukit_path.as_deref(),
        options.mobile || options.web,
    )?;
    let context = TemplateContext {
        app_name: options.name.clone(),
        app_crate: options.name.replace('-', "_"),
        app_env: options.name.replace('-', "_").to_ascii_uppercase(),
        template_version: TEMPLATE_VERSION.to_owned(),
        baukit_dependencies: dependency.cargo,
        baukit_typescript_dependencies: dependency.typescript,
        baukit_manifest: dependency.manifest,
        baukit_dependency_description: dependency.description,
        baukit_typescript_dependency_description: dependency.typescript_description,
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

    Ok(destination)
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
    typescript: String,
    manifest: String,
    description: String,
    typescript_description: String,
}

fn dependency_context(path: Option<&Path>, require_typescript: bool) -> Result<DependencyContext> {
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
        let names = [
            "baukit-config",
            "baukit-http",
            "baukit-openapi",
            "baukit-ops",
            "baukit-runtime",
            "baukit-telemetry",
            "baukit-test",
        ];
        let cargo = names
            .iter()
            .map(|name| format!("{name} = {{ path = \"{display}/crates/{name}\" }}"))
            .collect::<Vec<_>>()
            .join("\n");
        let repository = path
            .parent()
            .ok_or_else(|| anyhow!("Baukit Rust workspace has no repository parent"))?;
        let typescript_root = repository.join("typescript");
        if require_typescript {
            for package in EXPECTED_TYPESCRIPT_DEPENDENCIES {
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
        let typescript = EXPECTED_TYPESCRIPT_DEPENDENCIES
            .iter()
            .map(|package| {
                let directory = package.trim_start_matches("@baukit/");
                format!("    \"{package}\": \"file:{typescript_display}/packages/{directory}\"")
            })
            .collect::<Vec<_>>()
            .join(",\n");
        Ok(DependencyContext {
            cargo,
            typescript,
            manifest: format!("source = \"path\"\npath = \"{display}\""),
            description: format!("local path `{}`", path.display()),
            typescript_description: format!("local path `{}`", typescript_root.display()),
        })
    } else {
        let git = "https://github.com/patrickkoss/baukit.git";
        let tag = format!("v{TEMPLATE_VERSION}");
        let names = [
            "baukit-config",
            "baukit-http",
            "baukit-openapi",
            "baukit-ops",
            "baukit-runtime",
            "baukit-telemetry",
            "baukit-test",
        ];
        let cargo = names
            .iter()
            .map(|name| format!("{name} = {{ git = \"{git}\", tag = \"{tag}\" }}"))
            .collect::<Vec<_>>()
            .join("\n");
        let typescript = EXPECTED_TYPESCRIPT_DEPENDENCIES
            .iter()
            .map(|package| {
                let directory = package.trim_start_matches("@baukit/");
                format!(
                    "    \"{package}\": \"git+{git}#{tag}&path:typescript/packages/{directory}\""
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        Ok(DependencyContext {
            cargo,
            typescript,
            manifest: format!("source = \"git\"\ngit = \"{git}\"\ntag = \"{tag}\""),
            description: format!("git tag `{tag}` from `{git}`"),
            typescript_description: format!("git tag `{tag}` from `{git}`"),
        })
    }
}

fn render_product(
    context: &TemplateContext,
    options: &NewOptions,
) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut environment = Environment::new();
    environment.set_keep_trailing_newline(true);
    let mut rendered = BTreeMap::new();
    if options.backend {
        render_directory(&BACKEND_TEMPLATE, &environment, context, &mut rendered)?;
    }
    if options.mobile {
        render_directory(&MOBILE_TEMPLATE, &environment, context, &mut rendered)?;
    }
    if options.web {
        render_directory(&WEB_TEMPLATE, &environment, context, &mut rendered)?;
    }
    rendered.insert(
        PathBuf::from("baukit.toml"),
        render_manifest(context, options).into_bytes(),
    );
    Ok(rendered)
}

fn render_manifest(context: &TemplateContext, options: &NewOptions) -> String {
    format!(
        "schema_version = {MANIFEST_SCHEMA_VERSION}\n\
template_version = \"{}\"\n\
\n\
[app]\n\
name = \"{}\"\n\
\n\
[capabilities]\n\
backend = {}\n\
mobile = {}\n\
web = {}\n\
\n\
[dependencies.baukit]\n\
{}\n\
\n\
[openapi]\n\
schema = \"backend/openapi.json\"\n\
typescript = \"generated/openapi.d.ts\"\n",
        context.template_version,
        context.app_name,
        options.backend,
        options.mobile,
        options.web,
        context.baukit_manifest,
    )
}

fn render_directory(
    directory: &Dir<'_>,
    environment: &Environment<'_>,
    context: &TemplateContext,
    rendered: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    for file in directory.files() {
        let relative = file.path();
        let source = file
            .contents_utf8()
            .ok_or_else(|| anyhow!("template {} is not UTF-8", relative.display()))?;
        let name = relative.to_string_lossy();
        let mut output = environment.render_str(source, context!(context))?;
        if relative
            .file_name()
            .is_some_and(|name| name == "Cargo.toml")
            && context.app_crate != context.app_name
        {
            output = output.replace(
                &format!("{}-", context.app_crate),
                &format!("{}-", context.app_name),
            );
        }
        let output_path = PathBuf::from(name.replace("__app__", &context.app_name));
        rendered.insert(output_path, output.into_bytes());
    }
    for child in directory.dirs() {
        render_directory(child, environment, context, rendered)?;
    }
    Ok(())
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
    let manifest = read_manifest(root)?;
    let mut failures = Vec::new();
    let mut successes = Vec::new();
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
    if manifest.capabilities.backend {
        for expected in EXPECTED_BACKEND_FILES {
            let relative = expected.replace("__APP__", &manifest.app.name);
            if !root.join(&relative).is_file() {
                failures.push(format!("missing expected backend file `{relative}`"));
            }
        }
        let cargo = root.join("backend/Cargo.toml");
        if cargo.is_file() {
            let output = Command::new("cargo")
                .args([
                    "metadata",
                    "--manifest-path",
                    cargo.to_string_lossy().as_ref(),
                    "--format-version",
                    "1",
                    "--no-deps",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .context("could not run cargo metadata; install Rust from https://rustup.rs")?;
            if output.status.success() {
                successes.push("Cargo workspace and dependency manifests parse".to_owned());
            } else {
                failures.push(format!(
                    "Cargo workspace does not parse: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
    }
    if manifest.capabilities.mobile {
        validate_frontend_capability(
            root,
            "mobile",
            EXPECTED_MOBILE_FILES,
            &["expo", "react-native"],
            &mut successes,
            &mut failures,
        )?;
    }
    if manifest.capabilities.web {
        validate_frontend_capability(
            root,
            "web",
            EXPECTED_WEB_FILES,
            &["@tanstack/react-query", "vite"],
            &mut successes,
            &mut failures,
        )?;
    }
    match &manifest.dependencies.baukit {
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
        }
        BaukitDependency::Git { git, tag } => {
            let status = Command::new("git")
                .args(["ls-remote", "--exit-code", "--tags", git, tag])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .status();
            match status {
                Ok(status) if status.success() => {
                    successes.push(format!("Baukit git dependency resolves at tag {tag}"));
                }
                Ok(_) => failures.push(format!(
                    "Baukit git dependency `{git}` tag `{tag}` is not resolvable; check network access and the release tag"
                )),
                Err(error) => failures.push(format!(
                    "could not run git to resolve Baukit dependency: {error}"
                )),
            }
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

pub fn generate_openapi_client(root: &Path) -> Result<()> {
    let manifest = read_manifest(root)?;
    if !manifest.capabilities.backend {
        bail!("this product has no backend capability");
    }
    run_checked(
        Command::new("cargo").current_dir(root).args([
            "run",
            "--manifest-path",
            "backend/Cargo.toml",
            "-p",
            &format!("{}-bin", manifest.app.name),
            "--bin",
            "openapi",
            "--",
            &manifest.openapi.schema,
        ]),
        "OpenAPI export",
    )?;

    let output = root.join(&manifest.openapi.typescript);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let pnpm = command_exists("pnpm");
    let npx = command_exists("npx");
    if pnpm {
        run_checked(
            Command::new("pnpm").current_dir(root).args([
                "dlx",
                "openapi-typescript",
                &manifest.openapi.schema,
                "-o",
                &manifest.openapi.typescript,
            ]),
            "TypeScript client generation",
        )
    } else if npx {
        run_checked(
            Command::new("npx").current_dir(root).args([
                "--yes",
                "openapi-typescript",
                &manifest.openapi.schema,
                "-o",
                &manifest.openapi.typescript,
            ]),
            "TypeScript client generation",
        )
    } else {
        bail!(
            "OpenAPI schema was exported, but TypeScript generation needs Node.js and either pnpm or npx; install current Node LTS, then run `corepack enable` or install openapi-typescript"
        );
    }
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
