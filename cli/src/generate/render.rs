use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use include_dir::Dir;
use minijinja::{Environment, context};
use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::{
    BACKEND_TEMPLATE, COMMON_TEMPLATE, MCP_REMOTE_TEMPLATE, MOBILE_TEMPLATE, PLATFORM_TEMPLATE,
    WEB_TEMPLATE, WORKER_TEMPLATE, components::Components, dockerfile::render_dockerfile,
    generate::NewOptions, generate::manifest_source::render_manifest,
    generate::template_files::add_template_files_manifest, manifest::AuthProvider,
    manifest::Manifest,
};

/// Template variables for the selected auth provider.
#[derive(Debug)]
pub(crate) struct AuthContext(pub(crate) Option<AuthProvider>);

impl AuthContext {
    pub(crate) const fn enabled(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn is(&self, provider: AuthProvider) -> bool {
        self.0 == Some(provider)
    }

    const fn provider_name(&self) -> &'static str {
        match self.0 {
            Some(AuthProvider::Oidc) => "oidc",
            Some(AuthProvider::Clerk) => "clerk",
            Some(AuthProvider::Workos) => "workos",
            None => "",
        }
    }
}

impl Serialize for AuthContext {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(5))?;
        map.serialize_entry("auth_enabled", &self.enabled())?;
        map.serialize_entry("auth_oidc", &self.is(AuthProvider::Oidc))?;
        map.serialize_entry("auth_clerk", &self.is(AuthProvider::Clerk))?;
        map.serialize_entry("auth_workos", &self.is(AuthProvider::Workos))?;
        map.serialize_entry("auth_provider", self.provider_name())?;
        map.end()
    }
}

/// Which optional port groups `baukit.toml` declares explicitly.
#[derive(Debug, Serialize)]
pub(crate) struct DeclaredPorts {
    #[serde(rename = "dev_ports_declared")]
    pub(crate) dev: bool,
    #[serde(rename = "web_preview_declared")]
    pub(crate) web_preview: bool,
    #[serde(rename = "service_ports_declared")]
    pub(crate) service: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct TemplateContext {
    pub(crate) app_name: String,
    pub(crate) app_crate: String,
    pub(crate) app_env: String,
    pub(crate) template_version: String,
    pub(crate) baukit_dependencies: String,
    pub(crate) baukit_web_typescript_dependencies: String,
    pub(crate) baukit_web_typescript_dev_dependencies: String,
    pub(crate) baukit_mobile_typescript_dependencies: String,
    pub(crate) baukit_mobile_typescript_dev_dependencies: String,
    pub(crate) baukit_pnpm_overrides: String,
    pub(crate) baukit_manifest: String,
    pub(crate) baukit_dependency_description: String,
    pub(crate) baukit_typescript_dependency_description: String,
    pub(crate) product_description: String,
    #[serde(flatten)]
    pub(crate) components: Components,
    pub(crate) platform_migration: String,
    pub(crate) remote_mcp_client_scopes: &'static str,
    #[serde(flatten)]
    pub(crate) auth: AuthContext,
    pub(crate) quality_strict: bool,
    pub(crate) port_offset: u32,
    pub(crate) postgres_host_port: u16,
    pub(crate) api_host_port: u16,
    pub(crate) ops_host_port: u16,
    pub(crate) keycloak_host_port: u16,
    pub(crate) fake_provider_host_port: u16,
    pub(crate) redis_host_port: u16,
    pub(crate) web_port: u16,
    pub(crate) web_preview_port: u16,
    pub(crate) metro_port: Option<u16>,
    pub(crate) local_callback_port: u16,
    #[serde(flatten)]
    pub(crate) declared_ports: DeclaredPorts,
    pub(crate) api_container_port: u16,
    pub(crate) ops_container_port: u16,
}

pub(crate) type RenderedFiles = BTreeMap<PathBuf, Vec<u8>>;

struct Renderer<'a> {
    environment: Environment<'a>,
    context: &'a TemplateContext,
    rendered: RenderedFiles,
}

impl<'a> Renderer<'a> {
    fn new(context: &'a TemplateContext) -> Self {
        let mut environment = Environment::new();
        environment.set_keep_trailing_newline(true);
        Self {
            environment,
            context,
            rendered: BTreeMap::new(),
        }
    }

    fn directory(&mut self, directory: &Dir<'_>) -> Result<()> {
        render_directory(
            directory,
            &self.environment,
            self.context,
            &mut self.rendered,
            false,
        )
    }

    /// Renders a component template and, for products with auth, its `__auth__` overlay.
    fn component(&mut self, directory: &Dir<'_>) -> Result<()> {
        self.directory(directory)?;
        if self.context.auth.enabled()
            && let Some(overlay) = directory.get_dir("__auth__")
        {
            render_directory(
                overlay,
                &self.environment,
                self.context,
                &mut self.rendered,
                true,
            )?;
        }
        Ok(())
    }
}

pub(crate) fn render_product(
    context: &TemplateContext,
    options: &NewOptions,
) -> Result<RenderedFiles> {
    let components = options.components;
    let mut renderer = Renderer::new(context);
    renderer.directory(&COMMON_TEMPLATE)?;
    add_common_files(&mut renderer.rendered)?;
    if components.backend() {
        renderer.component(&BACKEND_TEMPLATE)?;
        if components.worker() {
            renderer.directory(&WORKER_TEMPLATE)?;
        }
    }
    if components.mobile() {
        renderer.component(&MOBILE_TEMPLATE)?;
        add_mobile_worker_scripts(&mut renderer.rendered)?;
    }
    if components.web() {
        renderer.component(&WEB_TEMPLATE)?;
    }
    if context.components.mcp() {
        renderer.directory(&MCP_REMOTE_TEMPLATE)?;
    }
    if context.components.platform() {
        renderer.component(&PLATFORM_TEMPLATE)?;
        add_platform_files(context, &mut renderer.rendered)?;
    }
    let mut rendered = renderer.rendered;
    let source = render_manifest(context, options);
    if components.backend() {
        let manifest: Manifest = toml::from_str(&source)?;
        rendered.insert(
            PathBuf::from("backend/Dockerfile"),
            render_dockerfile(&manifest)?.into_bytes(),
        );
    }
    rendered.insert(PathBuf::from("baukit.toml"), source.into_bytes());
    add_template_files_manifest(&mut rendered)?;
    Ok(rendered)
}

/// Symlinks written next to the rendered files, as `(link, target)` pairs relative to the product root.
pub(crate) const RENDERED_SYMLINKS: &[(&str, &str)] = &[("AGENTS.md", AGENT_GUIDE)];

const AGENT_GUIDE: &str = "CLAUDE.md";
const AGENT_RULES: &str = include_str!("../../../lint/agent-rules.md");
const AGENT_RULES_START: &str = "<!-- baukit:agent-rules:start -->\n";
const AGENT_RULES_END: &str = "<!-- baukit:agent-rules:end -->\n";

fn add_common_files(rendered: &mut RenderedFiles) -> Result<()> {
    let guide_path = Path::new(AGENT_GUIDE);
    let guide = rendered
        .get(guide_path)
        .ok_or_else(|| anyhow!("common template is missing {AGENT_GUIDE}"))?;
    let guide = with_agent_rules(std::str::from_utf8(guide)?)?;
    rendered.insert(guide_path.to_path_buf(), guide.into_bytes());
    rendered.insert(
        PathBuf::from("tests/fixtures/resource-budget-measurements-v1.json"),
        include_bytes!("../../../fixtures/limits/resource-budget-measurements-v1.json").to_vec(),
    );
    Ok(())
}

/// Fills the empty managed block in the agent guide with baukit's shared agent rules.
fn with_agent_rules(guide: &str) -> Result<String> {
    let empty_block = format!("{AGENT_RULES_START}{AGENT_RULES_END}");
    if guide.matches(&empty_block).count() != 1 {
        bail!("{AGENT_GUIDE} template must hold exactly one empty managed agent-rules block");
    }
    Ok(guide.replace(
        &empty_block,
        &format!("{AGENT_RULES_START}{AGENT_RULES}{AGENT_RULES_END}"),
    ))
}

/// The Expo web export builds its PWA worker with the web template's scripts.
fn add_mobile_worker_scripts(rendered: &mut RenderedFiles) -> Result<()> {
    for name in ["build-sw.mjs", "build-sw.test.mjs"] {
        let source = WEB_TEMPLATE
            .get_file(format!("web/scripts/{name}"))
            .ok_or_else(|| anyhow!("web template is missing {name}"))?;
        rendered.insert(
            PathBuf::from(format!("mobile/scripts/{name}")),
            source.contents().to_vec(),
        );
    }
    Ok(())
}

fn add_platform_files(context: &TemplateContext, rendered: &mut RenderedFiles) -> Result<()> {
    if context.components.worker() {
        rendered.remove(Path::new(&format!(
            "backend/crates/{}-bin/src/bin/platform-purge.rs",
            context.app_name
        )));
    }
    let schema_path = Path::new("backend/openapi.json");
    let mut schema: serde_json::Value = serde_json::from_slice(
        rendered
            .get(schema_path)
            .ok_or_else(|| anyhow!("backend OpenAPI schema is missing"))?,
    )?;
    schema
        .get_mut("paths")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| anyhow!("backend OpenAPI schema has no paths object"))?
        .insert(
            "/platform/inbound".to_owned(),
            serde_json::from_str(include_str!("../../resources/platform-openapi.json"))?,
        );
    rendered.insert(
        schema_path.to_path_buf(),
        format!("{}\n", serde_json::to_string_pretty(&schema)?).into_bytes(),
    );
    rendered.insert(
        PathBuf::from(&context.platform_migration),
        include_bytes!("../../../rust/crates/baukit-suite/migrations/004_platform.sql").to_vec(),
    );
    Ok(())
}

pub(crate) fn render_directory(
    directory: &Dir<'_>,
    environment: &Environment<'_>,
    context: &TemplateContext,
    rendered: &mut RenderedFiles,
    auth_overlay: bool,
) -> Result<()> {
    for file in directory.files() {
        let relative = file.path();
        if relative == Path::new("backend/Dockerfile.jinja") {
            continue;
        }
        if !context.auth.is(AuthProvider::Oidc)
            && !context.components.backend()
            && is_client_stack_only(relative)
        {
            continue;
        }
        if context.auth.is(AuthProvider::Oidc) && is_provider_shaped_realm_only(relative) {
            continue;
        }
        if is_python_cache_artifact(relative) {
            continue;
        }
        if is_auth_only(relative) && (!context.auth.enabled() || !auth_overlay) {
            continue;
        }
        if is_strict_only(relative) && !context.quality_strict {
            continue;
        }
        if is_backend_only(relative) && !context.components.backend() {
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
        if !context.auth.is(AuthProvider::Oidc)
            && !context.components.backend()
            && is_client_stack_only(child.path())
        {
            continue;
        }
        if is_python_cache_artifact(child.path()) {
            continue;
        }
        if is_auth_only(child.path()) && !auth_overlay {
            continue;
        }
        if is_strict_only(child.path()) && !context.quality_strict {
            continue;
        }
        if is_backend_only(child.path()) && !context.components.backend() {
            continue;
        }
        render_directory(child, environment, context, rendered, auth_overlay)?;
    }
    Ok(())
}

pub(crate) fn product_description(options: &NewOptions) -> String {
    let mut capabilities = Vec::new();
    if options.components.backend() {
        capabilities.push("Rust backend");
    }
    if options.components.worker() {
        capabilities.push("durable worker");
    }
    if options.components.web() {
        capabilities.push("web app");
    }
    if options.components.mobile() {
        capabilities.push("mobile app");
    }
    if options.components.mcp() {
        capabilities.push("MCP server");
    }
    format!("Baukit product with {}", capabilities.join(", "))
}

pub(crate) fn is_client_stack_only(path: &Path) -> bool {
    let value = path.to_string_lossy();
    value.contains("web/e2e/playwright.stack.config.ts") || value.contains("web/e2e/stack")
}

pub(crate) fn is_provider_shaped_realm_only(path: &Path) -> bool {
    path.ends_with("backend/tests/local_realm.rs")
        || path
            .parent()
            .is_some_and(|parent| parent.ends_with("src/sign-in"))
            && path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("local.") || name.starts_with("production.")
            })
        || path.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name.starts_with("check-production-bundle")
                || name == "sign-in-config.test.mjs"
                || name == "app-config.test.ts"
        })
}

pub(crate) fn is_auth_only(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__auth__")
}

pub(crate) fn is_strict_only(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__strict__")
}

pub(crate) fn is_backend_only(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__backend__")
}

pub(crate) fn is_python_cache_artifact(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__pycache__")
        || path.extension().is_some_and(|extension| extension == "pyc")
}

#[cfg(test)]
mod generator_tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use include_dir::{Dir, DirEntry, File};
    use minijinja::Environment;

    use super::{AuthContext, DeclaredPorts, TemplateContext, render_directory};
    use crate::REMOTE_MCP_CLIENT_SCOPES;
    use crate::{Component, Components};

    static PYTHON_CACHE_ENTRIES: &[DirEntry<'_>] = &[DirEntry::File(File::new(
        "__pycache__/template.cpython-313.pyc",
        &[0, 159, 146, 150],
    ))];
    static TEMPLATE_ENTRIES: &[DirEntry<'_>] = &[
        DirEntry::File(File::new("kept.txt.jinja", b"{{ context.app_name }}")),
        DirEntry::File(File::new("stray.pyc", &[0, 159, 146, 150])),
        DirEntry::Dir(Dir::new("__pycache__", PYTHON_CACHE_ENTRIES)),
    ];
    static TEMPLATE: Dir<'_> = Dir::new("", TEMPLATE_ENTRIES);

    #[test]
    fn embedded_python_cache_artifacts_do_not_reach_the_generated_tree() -> anyhow::Result<()> {
        let context = TemplateContext {
            app_name: "cache-test".to_owned(),
            app_crate: "cache_test".to_owned(),
            app_env: "CACHE_TEST".to_owned(),
            template_version: "0.0.0".to_owned(),
            baukit_dependencies: String::new(),
            baukit_web_typescript_dependencies: String::new(),
            baukit_web_typescript_dev_dependencies: String::new(),
            baukit_mobile_typescript_dependencies: String::new(),
            baukit_mobile_typescript_dev_dependencies: String::new(),
            baukit_pnpm_overrides: String::new(),
            baukit_manifest: String::new(),
            baukit_dependency_description: String::new(),
            baukit_typescript_dependency_description: String::new(),
            product_description: String::new(),
            components: Components::from([Component::Backend]),
            platform_migration: crate::PLATFORM_MIGRATION_PATH.to_owned(),
            remote_mcp_client_scopes: REMOTE_MCP_CLIENT_SCOPES,
            auth: AuthContext(None),
            quality_strict: false,
            port_offset: 0,
            postgres_host_port: 5432,
            api_host_port: 8080,
            ops_host_port: 9090,
            keycloak_host_port: 8081,
            fake_provider_host_port: 18081,
            redis_host_port: 6379,
            web_port: 5173,
            web_preview_port: 4173,
            metro_port: None,
            local_callback_port: crate::ports::LOCAL_CALLBACK_PORT,
            declared_ports: DeclaredPorts {
                dev: false,
                web_preview: false,
                service: false,
            },
            api_container_port: 8080,
            ops_container_port: 9090,
        };
        let mut rendered = BTreeMap::new();

        render_directory(
            &TEMPLATE,
            &Environment::new(),
            &context,
            &mut rendered,
            false,
        )?;

        assert_eq!(
            rendered,
            BTreeMap::from([(PathBuf::from("kept.txt"), b"cache-test".to_vec())])
        );
        Ok(())
    }
}
