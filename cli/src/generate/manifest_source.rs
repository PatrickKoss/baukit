use crate::{
    gate_settings,
    generate::NewOptions,
    generate::render::TemplateContext,
    manifest::{AnalyticsAdapter, AuthProvider, Capabilities, QualityProfile},
    manifest::{MANIFEST_SCHEMA_VERSION, default_backend_coverage_lines, default_webkit_repeats},
};

pub(crate) fn options_capabilities(options: &NewOptions) -> Capabilities {
    Capabilities {
        analytics: AnalyticsAdapter::Posthog,
        components: options.components,
        auth: options.auth,
    }
}

pub(crate) fn render_manifest(context: &TemplateContext, options: &NewOptions) -> String {
    let auth = match options.auth {
        Some(AuthProvider::Oidc) => "auth = \"oidc\"\n",
        Some(AuthProvider::Clerk) => "auth = \"clerk\"\n",
        Some(AuthProvider::Workos) => "auth = \"workos\"\n",
        None => "",
    };
    let port_offset = if options.port_offset == 0 {
        String::new()
    } else {
        format!("port_offset = {}\n", options.port_offset)
    };
    let mcp = if options.components.mcp() {
        "mcp = true\n"
    } else {
        ""
    };
    let platform_capability = if options.components.platform() {
        "platform = true\n"
    } else {
        ""
    };
    let platform_manifest = if options.components.platform() {
        format!(
            "\n[platform]\nmigration = {}\n",
            toml::Value::String(context.platform_migration.clone())
        )
    } else {
        String::new()
    };
    let optional_capabilities = format!("{mcp}{platform_capability}");
    let consumers = "[\"generated/openapi.d.ts\"]";
    let quality_extras = gate_settings::render_quality_extras(
        &context.app_name,
        &options_capabilities(options),
        options.quality == QualityProfile::Strict,
    );
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
openapi_compatibility = \"off\"\n\
{}\
\n\
[capabilities]\n\
analytics = \"posthog\"\n\
backend = {}\n\
worker = {}\n\
mobile = {}\n\
web = {}\n\
pwa = {}\n\
{}\
{}\
\n\
[dependencies.baukit]\n\
{}\n\
\n\
[openapi]\n\
schema = \"backend/openapi.json\"\n\
consumers = {}\n{}",
        context.template_version,
        port_offset,
        context.app_name,
        match options.quality {
            QualityProfile::Standard => "standard",
            QualityProfile::Strict => "strict",
        },
        default_backend_coverage_lines(),
        default_webkit_repeats(),
        quality_extras,
        options.components.backend(),
        options.components.worker(),
        options.components.mobile(),
        options.components.web(),
        options.components.pwa(),
        optional_capabilities,
        auth,
        context.baukit_manifest,
        consumers,
        platform_manifest,
    )
}
