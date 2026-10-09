use baukit_config::{ConfigLoader, Environment, LoadError};

const PRODUCT: &str = "{{ context.app_name }}";

pub fn config_loader(environment: Environment) -> Result<ConfigLoader, LoadError> {
{% if context.auth_enabled and context.mcp %}    Ok(ConfigLoader::new(PRODUCT, environment)?
        .environment_collection("auth.authorized_parties")
        .environment_collection("mcp.allowed_hosts")
        .environment_collection("mcp.allowed_origins"))
{% elif context.auth_enabled %}    Ok(ConfigLoader::new(PRODUCT, environment)?.environment_collection("auth.authorized_parties"))
{% else %}    ConfigLoader::new(PRODUCT, environment)
{% endif %}}
