use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use include_dir::Dir;
use minijinja::{Environment, context};
use serde::Serialize;

use crate::{
    BACKEND_TEMPLATE, COMMON_TEMPLATE, MCP_REMOTE_TEMPLATE, MOBILE_TEMPLATE, PLATFORM_TEMPLATE,
    WEB_TEMPLATE, WORKER_TEMPLATE, dockerfile::render_dockerfile, generate::NewOptions,
    generate::manifest_source::render_manifest, manifest::Manifest,
};

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
    pub(crate) baukit_manifest: String,
    pub(crate) baukit_dependency_description: String,
    pub(crate) baukit_typescript_dependency_description: String,
    pub(crate) product_description: String,
    pub(crate) backend: bool,
    pub(crate) worker: bool,
    pub(crate) mobile: bool,
    pub(crate) web: bool,
    pub(crate) pwa: bool,
    pub(crate) mcp: bool,
    pub(crate) platform: bool,
    pub(crate) platform_migration: String,
    pub(crate) remote_mcp_client_scopes: &'static str,
    pub(crate) auth_enabled: bool,
    pub(crate) auth_oidc: bool,
    pub(crate) auth_clerk: bool,
    pub(crate) auth_workos: bool,
    pub(crate) auth_provider: &'static str,
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
    pub(crate) dev_ports_declared: bool,
    pub(crate) web_preview_declared: bool,
    pub(crate) service_ports_declared: bool,
    pub(crate) api_container_port: u16,
    pub(crate) ops_container_port: u16,
}

pub(crate) fn render_product(
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
    rendered.insert(
        PathBuf::from("tests/fixtures/resource-budget-measurements-v1.json"),
        include_bytes!("../../../fixtures/limits/resource-budget-measurements-v1.json").to_vec(),
    );
    if options.backend {
        render_directory(
            &BACKEND_TEMPLATE,
            &environment,
            context,
            &mut rendered,
            false,
        )?;
        if context.auth_enabled
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
        if context.auth_enabled
            && let Some(overlay) = MOBILE_TEMPLATE.get_dir("__auth__")
        {
            render_directory(overlay, &environment, context, &mut rendered, true)?;
        }
    }
    if options.mobile {
        for name in ["build-sw.mjs", "build-sw.test.mjs"] {
            let source = WEB_TEMPLATE
                .get_file(format!("web/scripts/{name}"))
                .ok_or_else(|| anyhow!("web template is missing {name}"))?;
            rendered.insert(
                PathBuf::from(format!("mobile/scripts/{name}")),
                source.contents().to_vec(),
            );
        }
    }
    if options.web {
        render_directory(&WEB_TEMPLATE, &environment, context, &mut rendered, false)?;
        if context.auth_enabled
            && let Some(overlay) = WEB_TEMPLATE.get_dir("__auth__")
        {
            render_directory(overlay, &environment, context, &mut rendered, true)?;
        }
    }
    if context.mcp {
        render_directory(
            &MCP_REMOTE_TEMPLATE,
            &environment,
            context,
            &mut rendered,
            false,
        )?;
    }
    if context.platform {
        render_directory(
            &PLATFORM_TEMPLATE,
            &environment,
            context,
            &mut rendered,
            false,
        )?;
        if context.auth_enabled
            && let Some(overlay) = PLATFORM_TEMPLATE.get_dir("__auth__")
        {
            render_directory(overlay, &environment, context, &mut rendered, true)?;
        }
        if context.worker {
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
        schema["paths"]["/platform/inbound"] =
            serde_json::from_str(include_str!("../../resources/platform-openapi.json"))?;
        rendered.insert(
            schema_path.to_path_buf(),
            format!("{}\n", serde_json::to_string_pretty(&schema)?).into_bytes(),
        );
        rendered.insert(
            PathBuf::from(&context.platform_migration),
            include_bytes!("../../../rust/crates/baukit-suite/migrations/004_platform.sql")
                .to_vec(),
        );
    }
    let source = render_manifest(context, options);
    if options.backend {
        let manifest: Manifest = toml::from_str(&source)?;
        rendered.insert(
            PathBuf::from("backend/Dockerfile"),
            render_dockerfile(&manifest)?.into_bytes(),
        );
    }
    rendered.insert(PathBuf::from("baukit.toml"), source.into_bytes());
    Ok(rendered)
}

pub(crate) fn render_directory(
    directory: &Dir<'_>,
    environment: &Environment<'_>,
    context: &TemplateContext,
    rendered: &mut BTreeMap<PathBuf, Vec<u8>>,
    auth_overlay: bool,
) -> Result<()> {
    for file in directory.files() {
        let relative = file.path();
        if relative == Path::new("backend/Dockerfile.jinja") {
            continue;
        }
        if !context.auth_oidc && !context.backend && is_client_stack_only(relative) {
            continue;
        }
        if context.auth_oidc && is_provider_shaped_realm_only(relative) {
            continue;
        }
        if is_python_cache_artifact(relative) {
            continue;
        }
        if is_auth_only(relative) && (!context.auth_enabled || !auth_overlay) {
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
        if !context.auth_oidc && !context.backend && is_client_stack_only(child.path()) {
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
        if is_backend_only(child.path()) && !context.backend {
            continue;
        }
        render_directory(child, environment, context, rendered, auth_overlay)?;
    }
    Ok(())
}

pub(crate) fn product_description(options: &NewOptions) -> String {
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
    if options.mcp {
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

    use super::{TemplateContext, render_directory};
    use crate::REMOTE_MCP_CLIENT_SCOPES;

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
            baukit_manifest: String::new(),
            baukit_dependency_description: String::new(),
            baukit_typescript_dependency_description: String::new(),
            product_description: String::new(),
            backend: true,
            worker: false,
            mobile: false,
            web: false,
            pwa: false,
            mcp: false,
            platform: false,
            platform_migration: crate::PLATFORM_MIGRATION_PATH.to_owned(),
            remote_mcp_client_scopes: REMOTE_MCP_CLIENT_SCOPES,
            auth_enabled: false,
            auth_oidc: false,
            auth_clerk: false,
            auth_workos: false,
            auth_provider: "",
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
            dev_ports_declared: false,
            web_preview_declared: false,
            service_ports_declared: false,
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
