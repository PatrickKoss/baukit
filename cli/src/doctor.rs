use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::{
    COMMON_TEMPLATE, EXPECTED_MOBILE_AUTH_DEPENDENCIES, EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES,
    EXPECTED_WEB_AUTH_DEPENDENCIES, STRICT_GATE_TEMPLATE, TEMPLATE_VERSION,
    doctor_auth::{validate_auth_provider, validate_mobile_qa_build_flag},
    doctor_backend::{
        validate_jobs_migration, validate_migrations, validate_openapi_paths, validate_remote_mcp,
    },
    doctor_dependencies, doctor_files,
    doctor_frontend::{
        pwa_app_directory, validate_frontend_capability, validate_mobile_router_configuration,
        validate_pwa_worker_build, web_renders_expo_routes,
    },
    doctor_host::{
        DoctorHost, SystemDoctorHost, diagnose_docker, diagnose_ssh_agent,
        has_docker_build_targets, probe_git_dependency,
    },
    doctor_layout, doctor_platform,
    doctor_ports::{uses_bundled_keycloak, validate_port_configuration},
    gate_settings, identity,
    manifest::{
        AnalyticsAdapter, BaukitDependency, MANIFEST_SCHEMA_VERSION, Manifest,
        OpenApiCompatibility, QualityProfile, read_manifest,
    },
};

pub fn doctor(root: &Path) -> Result<Vec<String>> {
    doctor_with_host(root, &SystemDoctorHost)
}

pub(crate) fn doctor_with_host(root: &Path, host: &dyn DoctorHost) -> Result<Vec<String>> {
    let manifest = read_manifest(root)?;
    doctor_layout::validate_paths(root, &manifest)?;
    let mut failures = Vec::new();
    let mut successes = Vec::new();
    doctor_files::validate(root, &manifest, &mut successes, &mut failures)?;
    if manifest.quality.profile == QualityProfile::Strict
        && manifest.quality.full_stack_e2e
        && !root.join("scripts/full-stack-e2e.sh").is_file()
    {
        failures.push("quality.full_stack_e2e requires `scripts/full-stack-e2e.sh`".to_owned());
    }
    if !(1..=100).contains(&manifest.quality.backend_coverage_lines) {
        failures.push("quality.backend_coverage_lines must be between 1 and 100".to_owned());
    }
    let gate_steps = if manifest.quality.profile == QualityProfile::Strict {
        match gate_settings::list_gate_steps(root) {
            Some(Ok(steps)) => Some(steps),
            Some(Err(error)) => {
                failures.push(error);
                None
            }
            None => None,
        }
    } else {
        None
    };
    gate_settings::validate(
        &manifest.quality,
        &manifest.capabilities,
        gate_steps.as_ref(),
        &mut failures,
    );
    if let Some(gate) = &gate_steps {
        let template = COMMON_TEMPLATE
            .get_file(STRICT_GATE_TEMPLATE)
            .and_then(|file| file.contents_utf8())
            .context("the strict gate template is missing")?;
        match gate_settings::template_steps(template, &manifest.capabilities) {
            Ok(steps) => gate_settings::validate_template_steps(&steps, gate, &mut failures),
            Err(error) => failures.push(error),
        }
    }
    successes.extend(gate_settings::emulator_summary(
        &manifest.quality,
        &manifest.capabilities,
    ));
    if manifest.quality.webkit_repeats == 0 {
        failures.push("quality.webkit_repeats must be greater than zero".to_owned());
    }
    if manifest.quality.openapi_compatibility != OpenApiCompatibility::Off {
        if manifest.quality.profile != QualityProfile::Strict {
            failures.push("quality.openapi_compatibility requires the strict profile".to_owned());
        }
        if !manifest.capabilities.components.backend() {
            failures
                .push("quality.openapi_compatibility requires the backend capability".to_owned());
        }
    }
    if !manifest.capabilities.components.web() && !manifest.quality.critical_paths.is_empty() {
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
        } else if manifest.capabilities.components.web() && !root.join("web").join(path).is_file() {
            failures.push(format!(
                "critical path `web/{critical_path}` does not exist"
            ));
        }
    }
    if manifest.capabilities.components.pwa() && pwa_app_directory(&manifest.capabilities).is_none()
    {
        failures.push("the PWA capability requires the web or mobile capability".to_owned());
    }
    if manifest.capabilities.components.platform() && !manifest.capabilities.components.backend() {
        failures.push("the platform capability requires the backend capability".to_owned());
    }
    if manifest.capabilities.components.platform() != manifest.platform.is_some() {
        failures.push(
            "capabilities.platform and the [platform] migration must be set together".to_owned(),
        );
    }
    if manifest.capabilities.components.mcp() && !manifest.capabilities.components.backend() {
        failures.push("the MCP capability requires the backend capability".to_owned());
    }
    if manifest.capabilities.components.mcp() && manifest.capabilities.auth.is_none() {
        failures.push("remote MCP OAuth requires an auth provider".to_owned());
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
    validate_auth_provider(root, &manifest, &mut successes, &mut failures)?;
    validate_product_constants(root, &manifest, &mut successes, &mut failures)?;
    validate_port_configuration(root, &manifest, &mut successes, &mut failures)?;
    if manifest.capabilities.components.backend() {
        validate_openapi_paths(root, &manifest.openapi, &mut successes, &mut failures);
        doctor_layout::validate_backend_wiring(root, &manifest, &mut successes, &mut failures)?;
        validate_migrations(root, &manifest, &mut successes, &mut failures)?;
        if manifest.capabilities.components.worker() {
            validate_jobs_migration(root, &manifest, &mut successes, &mut failures)?;
        }
        if uses_bundled_keycloak(root, &manifest)? {
            doctor_layout::validate_keycloak_realm_tools(
                root,
                &manifest,
                host,
                &mut successes,
                &mut failures,
            )?;
        }
        if manifest.capabilities.components.platform() {
            doctor_platform::validate(root, &manifest, &mut successes, &mut failures)?;
        }
        if manifest.capabilities.components.mcp() {
            validate_remote_mcp(root, &manifest, &mut successes, &mut failures)?;
        }
    }
    for cargo in doctor_layout::cargo_workspaces(root, &manifest)? {
        let relative = cargo.strip_prefix(root)?.display();
        let args = vec![
            "metadata".to_owned(),
            "--manifest-path".to_owned(),
            cargo.to_string_lossy().into_owned(),
            "--format-version".to_owned(),
            "1".to_owned(),
            "--no-deps".to_owned(),
        ];
        let output = host.run_command("cargo", &args, None).with_context(|| {
            format!(
                "could not run cargo metadata for `{relative}`; install Rust from https://rustup.rs"
            )
        })?;
        if output.success {
            successes.push(format!(
                "Cargo workspace `{relative}` and dependency manifests parse"
            ));
        } else {
            failures.push(format!(
                "Cargo workspace `{relative}` does not parse: {}",
                output.stderr
            ));
        }
    }
    if manifest.capabilities.components.mobile() {
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
        if manifest.capabilities.analytics != AnalyticsAdapter::None {
            dependencies.push("@baukit/analytics-core");
        }
        if manifest.capabilities.analytics == AnalyticsAdapter::Posthog {
            dependencies.push("@baukit/analytics-posthog-native");
        }
        if manifest.capabilities.auth.is_some() {
            dependencies.extend(EXPECTED_MOBILE_AUTH_DEPENDENCIES);
        }
        validate_frontend_capability(root, "mobile", &dependencies, &mut successes, &mut failures)?;
        validate_mobile_router_configuration(root, &mut successes, &mut failures)?;
        validate_mobile_qa_build_flag(root, &mut failures)?;
        if manifest.capabilities.auth.is_some() {
            doctor_layout::validate_mobile_auth_wiring(root, &manifest, &mut failures)?;
        }
    }
    if manifest.capabilities.components.web() {
        let mut dependencies = vec!["@tanstack/react-query", "vite"];
        if manifest.capabilities.analytics != AnalyticsAdapter::None {
            dependencies.push("@baukit/analytics-core");
        }
        if manifest.capabilities.auth.is_some() {
            dependencies.extend(if web_renders_expo_routes(root)? {
                EXPECTED_MOBILE_AUTH_DEPENDENCIES
            } else {
                EXPECTED_WEB_AUTH_DEPENDENCIES
            });
        }
        validate_frontend_capability(root, "web", &dependencies, &mut successes, &mut failures)?;
    }
    if manifest.capabilities.components.pwa()
        && let Some(app_directory) = pwa_app_directory(&manifest.capabilities)
    {
        validate_pwa_worker_build(root, app_directory, &mut successes, &mut failures)?;
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
            doctor_dependencies::validate_registry_pins(
                root,
                version,
                &mut successes,
                &mut failures,
            )?;
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
        let mut findings = failures
            .iter()
            .map(|finding| format!("- {finding}"))
            .collect::<Vec<_>>();
        findings.extend(
            successes
                .into_iter()
                .filter(|finding| finding.starts_with("note: ")),
        );
        bail!(
            "doctor found {} problem(s):\n{}",
            failures.len(),
            findings.join("\n")
        )
    }
}

pub(crate) fn source_constant<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    source.split("const ").skip(1).find_map(|statement| {
        let (binding, initializer) = statement.split_once('=')?;
        let binding = binding.split(':').next()?.trim();
        if binding != name {
            return None;
        }
        Some(initializer.split_once(';')?.0.trim())
    })
}

pub(crate) fn quoted_string(value: &str) -> Option<&str> {
    ['\'', '"']
        .into_iter()
        .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
}

pub(crate) fn validate_product_constants(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    identity::validate(root, manifest, successes, failures)
}
