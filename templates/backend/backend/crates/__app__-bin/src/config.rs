use baukit_config::{ConfigLoader, Environment, LoadError};

const PRODUCT: &str = "{{ context.app_name }}";

pub fn config_loader(environment: Environment) -> Result<ConfigLoader, LoadError> {
{% if context.platform %}    let loader = ConfigLoader::new(PRODUCT, environment)?
        .environment_collection("platform.standalone_default.features");
{% if context.auth_enabled %}    let loader = loader.environment_collection("auth.authorized_parties");
{% endif %}{% if context.mcp %}    let loader = loader
        .environment_collection("mcp.allowed_hosts")
        .environment_collection("mcp.allowed_origins");
{% endif %}    Ok(loader)
{% elif context.auth_enabled and context.mcp %}    Ok(ConfigLoader::new(PRODUCT, environment)?
        .environment_collection("auth.authorized_parties")
        .environment_collection("mcp.allowed_hosts")
        .environment_collection("mcp.allowed_origins"))
{% elif context.auth_enabled %}    Ok(ConfigLoader::new(PRODUCT, environment)?.environment_collection("auth.authorized_parties"))
{% else %}    ConfigLoader::new(PRODUCT, environment)
{% endif %}}
