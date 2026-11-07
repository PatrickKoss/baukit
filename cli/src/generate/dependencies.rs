use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    EXPECTED_MOBILE_AUTH_DEPENDENCIES, EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES,
    EXPECTED_TYPESCRIPT_DEPENDENCIES, EXPECTED_WEB_AUTH_DEPENDENCIES,
    EXPECTED_WEB_AUTH_DEV_DEPENDENCIES, MOBILE_PURCHASES_DEPENDENCIES, TEMPLATE_VERSION,
    generate::NewOptions,
};

const BASE_CRATES: [&str; 9] = [
    "baukit-core",
    "baukit-config",
    "baukit-http",
    "baukit-openapi",
    "baukit-ops",
    "baukit-ratelimit",
    "baukit-runtime",
    "baukit-telemetry",
    "baukit-test",
];

/// Unreleased `@baukit/*` packages that other Baukit packages depend on. A `file:` install
/// resolves those dependencies from npm, so path mode pins them to the checkout with overrides.
const LOCAL_TRANSITIVE_PACKAGES: [&str; 1] = ["@baukit/type-guards"];

/// Lint presets every generated web and mobile client extends.
const ESLINT_CONFIG_PACKAGE: &str = "@baukit/eslint-config";

pub(crate) struct DependencyContext {
    pub(crate) cargo: String,
    pub(crate) web_typescript: String,
    pub(crate) web_dev_typescript: String,
    pub(crate) mobile_typescript: String,
    pub(crate) mobile_dev_typescript: String,
    pub(crate) pnpm_overrides: String,
    pub(crate) manifest: String,
    pub(crate) description: String,
    pub(crate) typescript_description: String,
}

/// The Baukit crates and TypeScript packages a product with these options depends on.
struct BaukitPackages {
    crates: Vec<&'static str>,
    web: Vec<&'static str>,
    web_dev: Vec<&'static str>,
    mobile: Vec<&'static str>,
    mobile_dev: Vec<&'static str>,
}

impl BaukitPackages {
    fn for_options(options: &NewOptions) -> Self {
        let components = options.components;
        let (mobile, web, pwa) = (components.mobile(), components.web(), components.pwa());
        let auth_enabled = options.auth.is_some();
        let mut web_packages = typescript_packages(false, false, web && auth_enabled);
        if pwa && web {
            web_packages.push("@baukit/pwa-web");
        }
        let mut mobile_packages = typescript_packages(mobile, mobile && auth_enabled, false);
        if pwa && mobile && !web {
            mobile_packages.push("@baukit/pwa-web");
        }
        let mobile_dev = if mobile {
            vec![ESLINT_CONFIG_PACKAGE]
        } else {
            Vec::new()
        };
        let mut web_dev = Vec::new();
        if web {
            web_dev.push(ESLINT_CONFIG_PACKAGE);
            if auth_enabled {
                web_dev.extend(EXPECTED_WEB_AUTH_DEV_DEPENDENCIES);
            }
        }
        Self {
            crates: baukit_crates(options),
            web: web_packages,
            web_dev,
            mobile: mobile_packages,
            mobile_dev,
        }
    }

    fn typescript(&self) -> impl Iterator<Item = &&'static str> {
        self.web
            .iter()
            .chain(&self.web_dev)
            .chain(&self.mobile)
            .chain(&self.mobile_dev)
    }
}

fn baukit_crates(options: &NewOptions) -> Vec<&'static str> {
    let components = options.components;
    let auth_enabled = options.auth.is_some();
    let mut names = BASE_CRATES.to_vec();
    if auth_enabled {
        names.extend([
            "baukit-auth",
            "baukit-credential-vault",
            "baukit-erasure",
        ]);
    }
    if components.worker() || auth_enabled {
        names.push("baukit-jobs");
    }
    if components.mcp() {
        names.push("baukit-mcp");
    }
    if components.platform() {
        names.extend(["baukit-suite", "baukit-egress"]);
    }
    names
}

pub(crate) fn dependency_context(options: &NewOptions) -> Result<DependencyContext> {
    let packages = BaukitPackages::for_options(options);
    match options.baukit_path.as_deref() {
        Some(path) => path_dependency_context(path, &packages, options),
        None => Ok(registry_dependency_context(&packages)),
    }
}

fn path_dependency_context(
    path: &Path,
    packages: &BaukitPackages,
    options: &NewOptions,
) -> Result<DependencyContext> {
    let path = path.canonicalize().with_context(|| {
        format!(
            "--baukit-path {} does not resolve; point it at the repository's rust directory",
            path.display()
        )
    })?;
    if !path.join("crates/baukit-config/Cargo.toml").is_file() {
        bail!(
            "--baukit-path {} is not a Baukit Rust workspace (missing crates/baukit-config/Cargo.toml)",
            path.display()
        );
    }
    let display = path.display().to_string().replace('\\', "\\\\");
    let cargo = packages
        .crates
        .iter()
        .map(|name| format!("{name} = {{ path = \"{display}/crates/{name}\" }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let typescript_root = path
        .parent()
        .ok_or_else(|| anyhow!("Baukit Rust workspace has no repository parent"))?
        .join("typescript");
    if options.components.mobile() || options.components.web() {
        ensure_local_typescript_packages(&path, &typescript_root, packages)?;
    }
    let typescript_display = typescript_root
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let local_specifier = |package: &str| {
        let directory = package.trim_start_matches("@baukit/");
        format!("file:{typescript_display}/packages/{directory}")
    };
    let render_typescript = |packages: &[&str]| render_package_entries(packages, local_specifier);
    let with_transitive = |packages: &[&'static str]| {
        let mut packages = packages.to_vec();
        if !packages.is_empty() {
            packages.extend(LOCAL_TRANSITIVE_PACKAGES);
        }
        packages
    };
    let pnpm_overrides = LOCAL_TRANSITIVE_PACKAGES
        .iter()
        .map(|package| format!("  '{package}': '{}'\n", local_specifier(package)))
        .collect();
    Ok(DependencyContext {
        cargo,
        web_typescript: render_typescript(&with_transitive(&packages.web)),
        web_dev_typescript: render_leading_entries(&render_typescript(&packages.web_dev)),
        mobile_typescript: render_typescript(&with_transitive(&packages.mobile)),
        mobile_dev_typescript: render_leading_entries(&render_typescript(&packages.mobile_dev)),
        pnpm_overrides,
        manifest: format!("source = \"path\"\npath = \"{display}\""),
        description: format!("local path `{}`", path.display()),
        typescript_description: format!("local path `{}`", typescript_root.display()),
    })
}

fn ensure_local_typescript_packages(
    path: &Path,
    typescript_root: &Path,
    packages: &BaukitPackages,
) -> Result<()> {
    for package in packages.typescript().chain(&LOCAL_TRANSITIVE_PACKAGES) {
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
    Ok(())
}

fn registry_dependency_context(packages: &BaukitPackages) -> DependencyContext {
    let version = TEMPLATE_VERSION;
    let cargo = packages
        .crates
        .iter()
        .map(|name| format!("{name} = \"{version}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let render_typescript =
        |packages: &[&str]| render_package_entries(packages, |_| version.to_owned());
    DependencyContext {
        cargo,
        web_typescript: render_typescript(&packages.web),
        web_dev_typescript: render_leading_entries(&render_typescript(&packages.web_dev)),
        mobile_typescript: render_typescript(&packages.mobile),
        mobile_dev_typescript: render_leading_entries(&render_typescript(&packages.mobile_dev)),
        pnpm_overrides: String::new(),
        manifest: format!("source = \"registry\"\nversion = \"{version}\""),
        description: format!("crates.io version `{version}`"),
        typescript_description: format!("npm version `{version}`"),
    }
}

fn render_package_entries(packages: &[&str], specifier: impl Fn(&str) -> String) -> String {
    packages
        .iter()
        .map(|package| format!("    \"{package}\": \"{}\"", specifier(package)))
        .collect::<Vec<_>>()
        .join(",\n")
}

pub(crate) fn render_leading_entries(entries: &str) -> String {
    if entries.is_empty() {
        String::new()
    } else {
        format!("{entries},\n")
    }
}

pub(crate) fn typescript_packages(
    mobile: bool,
    mobile_auth: bool,
    web_auth: bool,
) -> Vec<&'static str> {
    let mut packages = EXPECTED_TYPESCRIPT_DEPENDENCIES.to_vec();
    packages.insert(1, "@baukit/analytics-core");
    if mobile {
        packages.extend(EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES);
        packages.push("@baukit/analytics-posthog-native");
        packages.extend(MOBILE_PURCHASES_DEPENDENCIES);
    }
    if mobile_auth {
        packages.extend(EXPECTED_MOBILE_AUTH_DEPENDENCIES);
    }
    if web_auth {
        packages.extend(EXPECTED_WEB_AUTH_DEPENDENCIES);
    }
    packages
}
