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

const MOBILE_RUNTIME_DEPENDENCIES: &[&str] = &[
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

const WEB_RUNTIME_DEPENDENCIES: &[&str] = &["@tanstack/react-query", "vite"];

pub fn doctor(root: &Path) -> Result<Vec<String>> {
    doctor_with_host(root, &SystemDoctorHost)
}

/// Successful and failed doctor findings, in the order the checks report them.
#[derive(Default)]
struct Findings {
    successes: Vec<String>,
    failures: Vec<String>,
}

fn doctor_with_host(root: &Path, host: &dyn DoctorHost) -> Result<Vec<String>> {
    let manifest = read_manifest(root)?;
    doctor_layout::validate_paths(root, &manifest)?;
    let mut findings = Findings::default();
    let Findings {
        successes,
        failures,
    } = &mut findings;
    doctor_files::validate(root, &manifest, successes, failures)?;
    validate_quality_settings(root, &manifest, failures);
    validate_gate_steps(root, &manifest, failures)?;
    successes.extend(gate_settings::emulator_summary(
        &manifest.quality,
        &manifest.capabilities,
    ));
    validate_openapi_compatibility(&manifest, failures);
    validate_critical_paths(root, &manifest, failures);
    validate_capability_combinations(&manifest, failures);
    validate_versions(&manifest, successes, failures);
    validate_auth_provider(root, &manifest, successes, failures)?;
    identity::validate(root, &manifest, successes, failures)?;
    validate_port_configuration(root, &manifest, successes, failures)?;
    validate_backend(root, &manifest, host, successes, failures)?;
    validate_cargo_workspaces(root, &manifest, host, successes, failures)?;
    validate_mobile(root, &manifest, successes, failures)?;
    validate_web(root, &manifest, successes, failures)?;
    validate_pwa(root, &manifest, successes, failures)?;
    let ssh_agent_ready = validate_baukit_dependency(root, &manifest, host, successes, failures)?;
    validate_docker(root, host, ssh_agent_ready, successes, failures);
    findings.into_report()
}

impl Findings {
    fn into_report(self) -> Result<Vec<String>> {
        if self.failures.is_empty() {
            return Ok(self.successes);
        }
        let mut findings = self
            .failures
            .iter()
            .map(|finding| format!("- {finding}"))
            .collect::<Vec<_>>();
        findings.extend(
            self.successes
                .into_iter()
                .filter(|finding| finding.starts_with("note: ")),
        );
        bail!(
            "doctor found {} problem(s):\n{}",
            self.failures.len(),
            findings.join("\n")
        )
    }
}

fn validate_quality_settings(root: &Path, manifest: &Manifest, failures: &mut Vec<String>) {
    if manifest.quality.profile == QualityProfile::Strict
        && manifest.quality.full_stack_e2e
        && !root.join("scripts/full-stack-e2e.sh").is_file()
    {
        failures.push("quality.full_stack_e2e requires `scripts/full-stack-e2e.sh`".to_owned());
    }
    if !(1..=100).contains(&manifest.quality.backend_coverage_lines) {
        failures.push("quality.backend_coverage_lines must be between 1 and 100".to_owned());
    }
}

fn validate_gate_steps(root: &Path, manifest: &Manifest, failures: &mut Vec<String>) -> Result<()> {
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
        failures,
    );
    let Some(gate) = &gate_steps else {
        return Ok(());
    };
    let template = COMMON_TEMPLATE
        .get_file(STRICT_GATE_TEMPLATE)
        .and_then(|file| file.contents_utf8())
        .context("the strict gate template is missing")?;
    match gate_settings::template_steps(template, &manifest.capabilities) {
        Ok(steps) => gate_settings::validate_template_steps(&steps, gate, failures),
        Err(error) => failures.push(error),
    }
    Ok(())
}

fn validate_openapi_compatibility(manifest: &Manifest, failures: &mut Vec<String>) {
    if manifest.quality.webkit_repeats == 0 {
        failures.push("quality.webkit_repeats must be greater than zero".to_owned());
    }
    if manifest.quality.openapi_compatibility == OpenApiCompatibility::Off {
        return;
    }
    if manifest.quality.profile != QualityProfile::Strict {
        failures.push("quality.openapi_compatibility requires the strict profile".to_owned());
    }
    if !manifest.capabilities.components.backend() {
        failures.push("quality.openapi_compatibility requires the backend capability".to_owned());
    }
}

fn validate_critical_paths(root: &Path, manifest: &Manifest, failures: &mut Vec<String>) {
    let web = manifest.capabilities.components.web();
    if !web && !manifest.quality.critical_paths.is_empty() {
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
        } else if web && !root.join("web").join(path).is_file() {
            failures.push(format!(
                "critical path `web/{critical_path}` does not exist"
            ));
        }
    }
}

fn validate_capability_combinations(manifest: &Manifest, failures: &mut Vec<String>) {
    let capabilities = &manifest.capabilities;
    let components = capabilities.components;
    if components.pwa() && pwa_app_directory(capabilities).is_none() {
        failures.push("the PWA capability requires the web or mobile capability".to_owned());
    }
    if components.platform() && !components.backend() {
        failures.push("the platform capability requires the backend capability".to_owned());
    }
    if components.platform() != manifest.platform.is_some() {
        failures.push(
            "capabilities.platform and the [platform] migration must be set together".to_owned(),
        );
    }
    if components.mcp() && !components.backend() {
        failures.push("the MCP capability requires the backend capability".to_owned());
    }
    if components.mcp() && capabilities.auth.is_none() {
        failures.push("remote MCP OAuth requires an auth provider".to_owned());
    }
}

fn validate_versions(manifest: &Manifest, successes: &mut Vec<String>, failures: &mut Vec<String>) {
    if manifest.schema_version == MANIFEST_SCHEMA_VERSION {
        successes.push(format!(
            "manifest schema {MANIFEST_SCHEMA_VERSION} is current"
        ));
    } else {
        failures.push(format!(
            "baukit.toml schema_version is {}, but this CLI supports {}; regenerate or upgrade the manifest",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    if manifest.template_version == TEMPLATE_VERSION {
        successes.push(format!(
            "template version {TEMPLATE_VERSION} matches the CLI"
        ));
    } else {
        failures.push(format!(
            "product template version {} differs from CLI version {}; run the matching CLI or upgrade the product",
            manifest.template_version, TEMPLATE_VERSION
        ));
    }
}

fn validate_backend(
    root: &Path,
    manifest: &Manifest,
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let components = manifest.capabilities.components;
    if !components.backend() {
        return Ok(());
    }
    validate_openapi_paths(root, &manifest.openapi, successes, failures);
    doctor_layout::validate_backend_wiring(root, manifest, successes, failures)?;
    validate_migrations(root, manifest, successes, failures)?;
    if components.worker() {
        validate_jobs_migration(root, manifest, successes, failures)?;
    }
    if uses_bundled_keycloak(root, manifest)? {
        doctor_layout::validate_keycloak_realm_tools(root, manifest, host, successes, failures)?;
    }
    if components.platform() {
        doctor_platform::validate(root, manifest, successes, failures)?;
    }
    if components.mcp() {
        validate_remote_mcp(root, manifest, successes, failures)?;
    }
    Ok(())
}

fn validate_cargo_workspaces(
    root: &Path,
    manifest: &Manifest,
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    for cargo in doctor_layout::cargo_workspaces(root, manifest)? {
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
    Ok(())
}

fn validate_mobile(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let capabilities = &manifest.capabilities;
    if !capabilities.components.mobile() {
        return Ok(());
    }
    let mut dependencies = MOBILE_RUNTIME_DEPENDENCIES.to_vec();
    dependencies.extend(EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES);
    if capabilities.analytics != AnalyticsAdapter::None {
        dependencies.push("@baukit/analytics-core");
    }
    if capabilities.analytics == AnalyticsAdapter::Posthog {
        dependencies.push("@baukit/analytics-posthog-native");
    }
    if capabilities.auth.is_some() {
        dependencies.extend(EXPECTED_MOBILE_AUTH_DEPENDENCIES);
    }
    validate_frontend_capability(root, "mobile", &dependencies, successes, failures)?;
    validate_mobile_router_configuration(root, successes, failures)?;
    validate_mobile_qa_build_flag(root, failures)?;
    if capabilities.auth.is_some() {
        doctor_layout::validate_mobile_auth_wiring(root, manifest, failures)?;
    }
    Ok(())
}

fn validate_web(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let capabilities = &manifest.capabilities;
    if !capabilities.components.web() {
        return Ok(());
    }
    let mut dependencies = WEB_RUNTIME_DEPENDENCIES.to_vec();
    if capabilities.analytics != AnalyticsAdapter::None {
        dependencies.push("@baukit/analytics-core");
    }
    if capabilities.auth.is_some() {
        dependencies.extend(if web_renders_expo_routes(root)? {
            EXPECTED_MOBILE_AUTH_DEPENDENCIES
        } else {
            EXPECTED_WEB_AUTH_DEPENDENCIES
        });
    }
    validate_frontend_capability(root, "web", &dependencies, successes, failures)
}

fn validate_pwa(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    if !manifest.capabilities.components.pwa() {
        return Ok(());
    }
    let Some(app_directory) = pwa_app_directory(&manifest.capabilities) else {
        return Ok(());
    };
    validate_pwa_worker_build(root, app_directory, successes, failures)
}

/// Returns whether the SSH agent is ready, or `None` when the dependency source needs no SSH.
fn validate_baukit_dependency(
    root: &Path,
    manifest: &Manifest,
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<Option<bool>> {
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
            Ok(None)
        }
        BaukitDependency::Git { git, tag } => {
            let ready = diagnose_ssh_agent(host, successes, failures);
            probe_git_dependency(host, git, tag, successes, failures);
            Ok(Some(ready))
        }
        BaukitDependency::Registry { version } => {
            doctor_dependencies::validate_registry_pins(root, version, successes, failures)?;
            Ok(None)
        }
    }
}

fn validate_docker(
    root: &Path,
    host: &dyn DoctorHost,
    ssh_agent_ready: Option<bool>,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    if !has_docker_build_targets(root) || !diagnose_docker(host, successes, failures) {
        return;
    }
    match ssh_agent_ready {
        Some(true) => {
            successes.push("Docker BuildKit SSH forwarding prerequisites are present".to_owned());
        }
        Some(false) => failures.push(
            "Docker and BuildKit are available, but SSH forwarding is not ready; fix the SSH agent problem above"
                .to_owned(),
        ),
        None => successes.push(
            "Docker image builds are ready; registry and local path dependencies do not require SSH forwarding"
                .to_owned(),
        ),
    }
}
