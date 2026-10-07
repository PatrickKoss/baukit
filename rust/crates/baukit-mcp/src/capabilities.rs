use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use rmcp::ErrorData;
use serde_json::{Value, json};

use crate::{McpConfigError, Principal, ScopedTool, ToolService};

pub use rmcp::model::{
    GetPromptResult as PromptResult, Prompt, PromptArgument, PromptMessage, Resource,
    ResourceContents, ResourceTemplate, Role,
};

/// A resource and the grants required to list or read it.
#[derive(Clone, Debug)]
pub struct ScopedResource {
    pub resource: Resource,
    pub required_scopes: Vec<String>,
}

/// A URI template and the grants required to list or read its resources.
#[derive(Clone, Debug)]
pub struct ScopedResourceTemplate {
    pub template: ResourceTemplate,
    pub required_scopes: Vec<String>,
}

/// A prompt and the grants required to list or get it.
#[derive(Clone, Debug)]
pub struct ScopedPrompt {
    pub prompt: Prompt,
    pub required_scopes: Vec<String>,
}

pub type ResourceFuture<'a> = CapabilityFuture<'a, Vec<ResourceContents>>;
pub type PromptFuture<'a> = CapabilityFuture<'a, PromptResult>;
pub type CapabilityFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, CapabilityError>> + Send + 'a>>;

/// Resource definitions are fixed at registration. Product reads enforce ownership.
pub trait ResourceService: Send + Sync + 'static {
    fn list(&self) -> Vec<ScopedResource>;
    fn templates(&self) -> Vec<ScopedResourceTemplate>;
    fn read<'a>(&'a self, principal: &'a Principal, uri: &'a str) -> ResourceFuture<'a>;
}

pub trait PromptService: Send + Sync + 'static {
    fn list(&self) -> Vec<ScopedPrompt>;
    fn get<'a>(
        &'a self,
        principal: &'a Principal,
        name: &'a str,
        arguments: Value,
    ) -> PromptFuture<'a>;
}

/// Client-safe protocol errors. Messages and optional data must be redacted by the product.
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("{message}")]
    InvalidParams {
        message: String,
        data: Option<Value>,
    },
    #[error("{message}")]
    Internal {
        message: String,
        data: Option<Value>,
    },
}

impl From<CapabilityError> for ErrorData {
    fn from(error: CapabilityError) -> Self {
        match error {
            CapabilityError::InvalidParams { message, data } => Self::invalid_params(message, data),
            CapabilityError::Internal { message, data } => Self::internal_error(message, data),
        }
    }
}

/// Registers tools and optional resource and prompt services for the router.
pub struct McpServices {
    pub(crate) tools: Arc<dyn ToolService>,
    pub(crate) server_info: crate::Implementation,
    pub(crate) instructions: Option<String>,
    pub(crate) success_text_prefix: Option<String>,
    pub(crate) resources: Option<Arc<dyn ResourceService>>,
    pub(crate) prompts: Option<Arc<dyn PromptService>>,
}

impl McpServices {
    pub fn new(tools: Arc<dyn ToolService>) -> Self {
        Self {
            tools,
            server_info: crate::Implementation::new("baukit-mcp", env!("CARGO_PKG_VERSION")),
            instructions: None,
            success_text_prefix: None,
            resources: None,
            prompts: None,
        }
    }

    /// Sets the product name, version and optional display metadata in initialize.
    pub fn with_server_info(mut self, server_info: crate::Implementation) -> Self {
        self.server_info = server_info;
        self
    }

    /// Sets the instructions returned during initialize.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Prepends text to successful tool content. Structured content and errors are unchanged.
    /// Include any required separator in the prefix.
    pub fn with_success_text_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.success_text_prefix = Some(prefix.into());
        self
    }

    pub fn with_resources(mut self, resources: Arc<dyn ResourceService>) -> Self {
        self.resources = Some(resources);
        self
    }

    pub fn with_prompts(mut self, prompts: Arc<dyn PromptService>) -> Self {
        self.prompts = Some(prompts);
        self
    }
}

pub(crate) fn validate_scopes(scopes: &[String]) -> Result<(), McpConfigError> {
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| !crate::server::valid_scope(scope))
    {
        return Err(McpConfigError::Invalid(
            "each definition requires nonempty OAuth scopes",
        ));
    }
    Ok(())
}

fn unique<'a>(values: impl Iterator<Item = &'a str>) -> Result<(), McpConfigError> {
    let mut seen = BTreeSet::new();
    if values
        .into_iter()
        .any(|value| value.is_empty() || !seen.insert(value))
    {
        return Err(McpConfigError::Invalid(
            "definition names and URIs must be nonempty and unique",
        ));
    }
    Ok(())
}

pub(crate) fn validate_resources(
    resources: &[ScopedResource],
    templates: &[ScopedResourceTemplate],
) -> Result<(), McpConfigError> {
    unique(resources.iter().map(|r| r.resource.uri.as_str()))?;
    unique(resources.iter().map(|r| r.resource.name.as_str()))?;
    unique(templates.iter().map(|r| r.template.uri_template.as_str()))?;
    unique(templates.iter().map(|r| r.template.name.as_str()))?;
    for resource in resources {
        validate_scopes(&resource.required_scopes)?;
        if url::Url::parse(&resource.resource.uri).is_err()
            || resource.resource.uri.contains(['{', '}'])
        {
            return Err(McpConfigError::Invalid(
                "resource URI must be absolute without template variables",
            ));
        }
    }
    for template in templates {
        validate_scopes(&template.required_scopes)?;
        validate_template(&template.template.uri_template)?;
    }
    Ok(())
}

fn validate_template(template: &str) -> Result<(), McpConfigError> {
    if url::Url::parse(template).is_err() {
        return Err(McpConfigError::Invalid(
            "resource template must be an absolute URI",
        ));
    }
    let mut variables = BTreeSet::new();
    for segment in template.split('/') {
        if !segment.contains(['{', '}']) {
            continue;
        }
        let valid = segment
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .is_some_and(|name| {
                !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && variables.insert(name)
            });
        if !valid {
            return Err(McpConfigError::Invalid(
                "resource templates support unique {name} path segments",
            ));
        }
    }
    Ok(())
}

pub(crate) fn template_matches(template: &str, uri: &str) -> bool {
    let mut parts = uri.split('/');
    template.split('/').all(|segment| {
        parts.next().is_some_and(|part| {
            if segment.starts_with('{') {
                !part.is_empty() && !part.contains(['?', '#', '{', '}'])
            } else {
                segment == part
            }
        })
    }) && parts.next().is_none()
}

pub(crate) fn validate_prompts(prompts: &[ScopedPrompt]) -> Result<(), McpConfigError> {
    unique(prompts.iter().map(|p| p.prompt.name.as_str()))?;
    for prompt in prompts {
        validate_scopes(&prompt.required_scopes)?;
        unique(
            prompt
                .prompt
                .arguments
                .iter()
                .flatten()
                .map(|a| a.name.as_str()),
        )?;
    }
    Ok(())
}

/// Exports resource metadata, URI templates and their grants for committed drift checks.
pub fn resource_schema(
    resources: &[ScopedResource],
    templates: &[ScopedResourceTemplate],
) -> Result<Value, McpConfigError> {
    validate_resources(resources, templates)?;
    Ok(json!({
        "resources": resources.iter().map(|r| &r.resource).collect::<Vec<_>>(),
        "templates": templates.iter().map(|r| &r.template).collect::<Vec<_>>(),
        "scopes": resources.iter().map(|r| (&r.resource.uri, &r.required_scopes)).collect::<std::collections::BTreeMap<_, _>>(),
        "templateScopes": templates.iter().map(|r| (&r.template.uri_template, &r.required_scopes)).collect::<std::collections::BTreeMap<_, _>>()
    }))
}

/// Exports prompt metadata, arguments and grants for committed drift checks.
pub fn prompt_schema(prompts: &[ScopedPrompt]) -> Result<Value, McpConfigError> {
    validate_prompts(prompts)?;
    Ok(json!({
        "prompts": prompts.iter().map(|p| &p.prompt).collect::<Vec<_>>(),
        "scopes": prompts.iter().map(|p| (&p.prompt.name, &p.required_scopes)).collect::<std::collections::BTreeMap<_, _>>()
    }))
}

/// Exports all product definitions, including empty optional registries.
pub fn capability_schema(
    tools: &[ScopedTool],
    resources: &[ScopedResource],
    templates: &[ScopedResourceTemplate],
    prompts: &[ScopedPrompt],
) -> Result<Value, McpConfigError> {
    Ok(
        json!({"tools": crate::tool_schema(tools)?, "resources": resource_schema(resources, templates)?, "prompts": prompt_schema(prompts)?}),
    )
}

/// Exports the definitions registered by a product's composed services.
pub fn service_schema(services: &McpServices) -> Result<Value, McpConfigError> {
    let resources = &services.resources;
    let prompts = &services.prompts;
    capability_schema(
        &services.tools.tools(),
        &resources.as_ref().map_or_else(Vec::new, |s| s.list()),
        &resources.as_ref().map_or_else(Vec::new, |s| s.templates()),
        &prompts.as_ref().map_or_else(Vec::new, |s| s.list()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource() -> ScopedResource {
        ScopedResource {
            resource: Resource::new("product://guide", "guide").with_mime_type("application/json"),
            required_scopes: vec!["content:read".into()],
        }
    }
    fn template() -> ScopedResourceTemplate {
        ScopedResourceTemplate {
            template: ResourceTemplate::new("product://items/{id}", "item")
                .with_title("Owned item"),
            required_scopes: vec!["items:read".into()],
        }
    }
    fn prompt() -> ScopedPrompt {
        ScopedPrompt {
            prompt: Prompt::new(
                "recommend",
                Some("Practice"),
                Some(vec![PromptArgument::new("language").with_required(true)]),
            ),
            required_scopes: vec!["learning:read".into()],
        }
    }

    #[test]
    fn drift_preserves_resource_metadata_templates_prompt_arguments_and_scopes() {
        let resources = resource_schema(&[resource()], &[template()]).expect("resources");
        assert_eq!(resources["resources"][0]["mimeType"], "application/json");
        assert_eq!(
            resources["scopes"]["product://guide"],
            json!(["content:read"])
        );
        assert_eq!(
            resources["templates"][0]["uriTemplate"],
            "product://items/{id}"
        );
        assert_eq!(resources["templates"][0]["title"], "Owned item");
        assert_eq!(
            resources["templateScopes"]["product://items/{id}"],
            json!(["items:read"])
        );
        let prompts = prompt_schema(&[prompt()]).expect("prompts");
        assert_eq!(
            prompts["prompts"][0]["arguments"][0],
            json!({"name":"language","required":true})
        );
        assert_eq!(prompts["scopes"]["recommend"], json!(["learning:read"]));
        let mut changed = template();
        changed.template.uri_template = "product://content/{id}".into();
        assert_ne!(
            resources,
            resource_schema(&[resource()], &[changed]).expect("changed template")
        );
        let mut changed = prompt();
        changed.prompt.arguments = Some(vec![PromptArgument::new("locale").with_required(false)]);
        assert_ne!(prompts, prompt_schema(&[changed]).expect("changed prompt"));
    }

    #[test]
    fn ambiguous_definitions_and_invalid_scopes_are_rejected() {
        assert!(resource_schema(&[resource(), resource()], &[]).is_err());
        assert!(resource_schema(&[], &[template(), template()]).is_err());
        assert!(prompt_schema(&[prompt(), prompt()]).is_err());
        for scopes in [
            vec![],
            vec!["".into()],
            vec!["read write".into()],
            vec!["quote\"".into()],
        ] {
            let mut resource = resource();
            resource.required_scopes = scopes.clone();
            let mut template = template();
            template.required_scopes = scopes.clone();
            let mut prompt = prompt();
            prompt.required_scopes = scopes;
            assert!(resource_schema(&[resource], &[]).is_err());
            assert!(resource_schema(&[], &[template]).is_err());
            assert!(prompt_schema(&[prompt]).is_err());
        }
        let mut prompt = prompt();
        prompt.prompt.arguments = Some(vec![
            PromptArgument::new("language"),
            PromptArgument::new("language"),
        ]);
        assert!(prompt_schema(&[prompt]).is_err());
        for uri in ["relative", "product://items/{id}"] {
            let mut resource = resource();
            resource.resource.uri = uri.into();
            assert!(resource_schema(&[resource], &[]).is_err());
        }
        for uri in [
            "relative/{id}",
            "product://items/{+id}",
            "product://items/{id}/{id}",
            "product://items/{id",
            "product://items/{id}.json",
        ] {
            let mut template = template();
            template.template.uri_template = uri.into();
            assert!(resource_schema(&[], &[template]).is_err(), "{uri}");
        }
    }

    #[test]
    fn template_matching_is_exact_and_requires_nonempty_segments() {
        for uri in ["product://items/42", "product://items/caf%C3%A9"] {
            assert!(template_matches("product://items/{id}", uri));
        }
        for uri in [
            "other://items/42",
            "product://items/",
            "product://items/42/extra",
            "product://items/42?other=1",
            "product://items/42#fragment",
            "product://items/{id}",
        ] {
            assert!(!template_matches("product://items/{id}", uri), "{uri}");
        }
    }

    #[test]
    fn safe_capability_errors_keep_protocol_codes_and_product_details() {
        let error: ErrorData = CapabilityError::InvalidParams {
            message: "Item not found".into(),
            data: Some(json!({"status":404,"code":"not_found"})),
        }
        .into();
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(error.message, "Item not found");
        assert_eq!(error.data, Some(json!({"status":404,"code":"not_found"})));
        let error: ErrorData = CapabilityError::Internal {
            message: "Items unavailable".into(),
            data: None,
        }
        .into();
        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "Items unavailable");
        assert_eq!(error.data, None);
    }
}
