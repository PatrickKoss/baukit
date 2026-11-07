use anyhow::{Context, Result, anyhow, bail};

use crate::{
    EXPECTED_MOBILE_AUTH_DEPENDENCIES, EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES,
    EXPECTED_TYPESCRIPT_DEPENDENCIES, EXPECTED_WEB_AUTH_DEPENDENCIES,
    EXPECTED_WEB_AUTH_DEV_DEPENDENCIES, MOBILE_PURCHASES_DEPENDENCIES, TEMPLATE_VERSION,
    generate::NewOptions,
};

pub(crate) struct DependencyContext {
    pub(crate) cargo: String,
    pub(crate) web_typescript: String,
    pub(crate) web_dev_typescript: String,
    pub(crate) mobile_typescript: String,
    pub(crate) manifest: String,
    pub(crate) description: String,
    pub(crate) typescript_description: String,
}

pub(crate) fn dependency_context(options: &NewOptions) -> Result<DependencyContext> {
    let path = options.baukit_path.as_deref();
    let (mobile, web, pwa) = (options.mobile, options.web, options.pwa);
    let auth_enabled = options.auth.is_some();
    let (worker, mcp, platform) = (options.worker, options.mcp, options.platform);
    let mut web_packages = typescript_packages(false, false, web && auth_enabled);
    if pwa && web {
        web_packages.push("@baukit/pwa-web");
    }
    let web_dev_packages: &[&str] = if web && auth_enabled {
        EXPECTED_WEB_AUTH_DEV_DEPENDENCIES
    } else {
        &[]
    };
    let mut mobile_packages = typescript_packages(mobile, mobile && auth_enabled, false);
    if pwa && mobile && !web {
        mobile_packages.push("@baukit/pwa-web");
    }
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
            "baukit-core",
            "baukit-config",
            "baukit-http",
            "baukit-openapi",
            "baukit-ops",
            "baukit-runtime",
            "baukit-telemetry",
            "baukit-test",
        ];
        if auth_enabled {
            names.extend(["baukit-auth", "baukit-erasure", "baukit-ratelimit"]);
        }
        if worker || auth_enabled {
            names.push("baukit-jobs");
        }
        if mcp {
            names.push("baukit-mcp");
        }
        if platform {
            names.extend(["baukit-suite", "baukit-egress"]);
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
            for package in web_packages
                .iter()
                .chain(web_dev_packages)
                .chain(&mobile_packages)
            {
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
            web_dev_typescript: render_leading_entries(&render_typescript(web_dev_packages)),
            mobile_typescript: render_typescript(&mobile_packages),
            manifest: format!("source = \"path\"\npath = \"{display}\""),
            description: format!("local path `{}`", path.display()),
            typescript_description: format!("local path `{}`", typescript_root.display()),
        })
    } else {
        let version = TEMPLATE_VERSION;
        let mut names = vec![
            "baukit-core",
            "baukit-config",
            "baukit-http",
            "baukit-openapi",
            "baukit-ops",
            "baukit-runtime",
            "baukit-telemetry",
            "baukit-test",
        ];
        if auth_enabled {
            names.extend(["baukit-auth", "baukit-erasure", "baukit-ratelimit"]);
        }
        if worker || auth_enabled {
            names.push("baukit-jobs");
        }
        if mcp {
            names.push("baukit-mcp");
        }
        if platform {
            names.extend(["baukit-suite", "baukit-egress"]);
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
            web_dev_typescript: render_leading_entries(&render_typescript(web_dev_packages)),
            mobile_typescript: render_typescript(&mobile_packages),
            manifest: format!("source = \"registry\"\nversion = \"{version}\""),
            description: format!("crates.io version `{version}`"),
            typescript_description: format!("npm version `{version}`"),
        })
    }
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
