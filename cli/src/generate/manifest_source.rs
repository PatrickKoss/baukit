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
    let port_offset = if options.port_offset == 0 {
        String::new()
    } else {
        format!("port_offset = {}\n", options.port_offset)
    };
    let platform_manifest = if options.components.platform() {
        format!(
            "\n[platform]\nmigration = {}\n",
            toml::Value::String(context.platform_migration.clone())
        )
    } else {
        String::new()
    };
    let consumers = "[\"generated/openapi.d.ts\"]";
    format!(
        "schema_version = {MANIFEST_SCHEMA_VERSION}\n\
template_version = \"{}\"\n\
{}\
\n\
[app]\n\
name = \"{}\"\n\
\n\
{}\
\n\
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
        quality_section(context, options),
        capabilities_section(options),
        context.baukit_manifest,
        consumers,
        platform_manifest,
    )
}

fn quality_section(context: &TemplateContext, options: &NewOptions) -> String {
    let quality_extras = gate_settings::render_quality_extras(
        &context.app_name,
        &options_capabilities(options),
        options.quality == QualityProfile::Strict,
    );
    let profile = match options.quality {
        QualityProfile::Standard => "standard",
        QualityProfile::Strict => "strict",
    };
    format!(
        "[quality]\n\
profile = \"{profile}\"\n\
backend_coverage_lines = {}\n\
critical_paths = []\n\
webkit_repeats = {}\n\
full_stack_e2e = false\n\
openapi_compatibility = \"off\"\n\
{quality_extras}",
        default_backend_coverage_lines(),
        default_webkit_repeats(),
    )
}

fn capabilities_section(options: &NewOptions) -> String {
    let components = options.components;
    let auth = match options.auth {
        Some(AuthProvider::Oidc) => "auth = \"oidc\"\n",
        Some(AuthProvider::Clerk) => "auth = \"clerk\"\n",
        Some(AuthProvider::Workos) => "auth = \"workos\"\n",
        None => "",
    };
    let mcp = if components.mcp() { "mcp = true\n" } else { "" };
    let platform = if components.platform() {
        "platform = true\n"
    } else {
        ""
    };
    format!(
        "[capabilities]\n\
analytics = \"posthog\"\n\
backend = {}\n\
worker = {}\n\
mobile = {}\n\
web = {}\n\
pwa = {}\n\
{mcp}{platform}{auth}",
        components.backend(),
        components.worker(),
        components.mobile(),
        components.web(),
        components.pwa(),
    )
}
