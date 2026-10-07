use std::{borrow::Cow, collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
        Tool, ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::{Value, json};

use crate::capabilities::{
    ScopedPrompt, ScopedResource, ScopedResourceTemplate, template_matches, validate_prompts,
    validate_resources,
};
use crate::{McpConfigError, McpServices, Principal, PromptService, ResourceService};

const MAX_TOOL_NAME_BYTES: usize = 128;

/// A product tool and the OAuth grants required to call it.
#[derive(Clone, Debug)]
pub struct ScopedTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub required_scopes: Vec<String>,
    pub read_only: bool,
    /// Optional hints override the defaults derived from `read_only`.
    pub annotations: ToolAnnotations,
}

/// Asynchronous product service result without transport types.
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + 'a>>;

/// Product service port. Implementations receive only verified identity and JSON arguments.
pub trait ToolService: Send + Sync + 'static {
    fn tools(&self) -> Vec<ScopedTool>;
    fn call<'a>(
        &'a self,
        principal: &'a Principal,
        name: &'a str,
        arguments: Value,
    ) -> ToolFuture<'a>;
}

/// Safe error returned to clients. Keep database and provider errors private.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    pub code: String,
    pub message: String,
    content: ErrorContent,
}

#[derive(Debug)]
enum ErrorContent {
    Default,
    Structured(Value),
    TextOnly,
}

impl ToolError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            content: ErrorContent::Default,
        }
    }

    /// Replaces the default {code,message} payload. The message is the text content.
    pub fn with_structured_content(mut self, value: Value) -> Self {
        self.content = ErrorContent::Structured(value);
        self
    }

    /// Returns an isError result with text content and no structured content.
    pub fn text_only(mut self) -> Self {
        self.content = ErrorContent::TextOnly;
        self
    }

    fn into_result(self) -> CallToolResult {
        let mut result = match self.content {
            ErrorContent::Default => {
                return CallToolResult::structured_error(
                    json!({"code":self.code,"message":self.message}),
                );
            }
            ErrorContent::Structured(value) => CallToolResult::structured_error(value),
            ErrorContent::TextOnly => CallToolResult::error(Vec::new()),
        };
        result.content = vec![rmcp::model::ContentBlock::text(self.message)];
        result
    }
}

pub(crate) struct RegisteredServices {
    pub service: Arc<dyn ToolService>,
    pub server_info: Implementation,
    pub instructions: Option<String>,
    pub success_text_prefix: Option<String>,
    pub definitions: Vec<ScopedTool>,
    pub protocol_tools: Vec<Tool>,
    pub resources: Option<Arc<dyn ResourceService>>,
    pub resource_definitions: Vec<ScopedResource>,
    pub templates: Vec<ScopedResourceTemplate>,
    pub prompts: Option<Arc<dyn PromptService>>,
    pub prompt_definitions: Vec<ScopedPrompt>,
}

impl RegisteredServices {
    pub fn new(services: McpServices) -> Result<Self, McpConfigError> {
        let McpServices {
            tools: service,
            server_info,
            instructions,
            success_text_prefix,
            resources,
            prompts,
        } = services;
        let definitions = service.tools();
        let protocol_tools = compile_tools(&definitions)?;
        let resource_definitions = resources.as_ref().map_or_else(Vec::new, |s| s.list());
        let templates = resources.as_ref().map_or_else(Vec::new, |s| s.templates());
        validate_resources(&resource_definitions, &templates)?;
        let prompt_definitions = prompts.as_ref().map_or_else(Vec::new, |s| s.list());
        validate_prompts(&prompt_definitions)?;
        Ok(Self {
            server_info,
            instructions,
            success_text_prefix,
            resources,
            resource_definitions,
            templates,
            prompts,
            prompt_definitions,
            service,
            definitions,
            protocol_tools,
        })
    }

    pub fn scopes(&self) -> Vec<String> {
        self.definitions
            .iter()
            .flat_map(|tool| tool.required_scopes.iter().cloned())
            .chain(
                self.resource_definitions
                    .iter()
                    .flat_map(|r| r.required_scopes.iter().cloned()),
            )
            .chain(
                self.templates
                    .iter()
                    .flat_map(|r| r.required_scopes.iter().cloned()),
            )
            .chain(
                self.prompt_definitions
                    .iter()
                    .flat_map(|p| p.required_scopes.iter().cloned()),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn required_scopes(&self, message: &Value) -> Option<Vec<String>> {
        match message.get("method")?.as_str()? {
            "tools/call" => self
                .tool(message.pointer("/params/name")?.as_str()?)
                .map(|tool| tool.required_scopes.clone()),
            "resources/read" => self.resource_scopes(message.pointer("/params/uri")?.as_str()?),
            "prompts/get" => {
                let name = message.pointer("/params/name")?.as_str()?;
                self.prompt_definitions
                    .iter()
                    .find(|definition| definition.prompt.name == name)
                    .map(|definition| definition.required_scopes.clone())
            }
            _ => None,
        }
    }

    fn resource_scopes(&self, uri: &str) -> Option<Vec<String>> {
        let scopes = self
            .resource_definitions
            .iter()
            .filter(|resource| resource.resource.uri == uri)
            .map(|resource| &resource.required_scopes)
            .chain(
                self.templates
                    .iter()
                    .filter(|template| template_matches(&template.template.uri_template, uri))
                    .map(|template| &template.required_scopes),
            )
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        (!scopes.is_empty()).then(|| scopes.into_iter().collect())
    }

    pub fn tool(&self, name: &str) -> Option<&ScopedTool> {
        self.definitions.iter().find(|tool| tool.name == name)
    }
}

pub(crate) fn valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && scope
            .bytes()
            .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
}

fn compile_tools(definitions: &[ScopedTool]) -> Result<Vec<Tool>, McpConfigError> {
    let mut names = BTreeSet::new();
    let mut protocol_tools = Vec::new();
    for definition in definitions {
        if definition.name.is_empty()
            || definition.name.len() > MAX_TOOL_NAME_BYTES
            || !names.insert(&definition.name)
        {
            return Err(McpConfigError::Invalid(
                "tool names must be nonempty, unique, and at most 128 bytes",
            ));
        }
        if definition.required_scopes.is_empty()
            || definition
                .required_scopes
                .iter()
                .any(|scope| !valid_scope(scope))
        {
            return Err(McpConfigError::Invalid(
                "each tool requires nonempty OAuth scopes",
            ));
        }
        let schema = definition
            .input_schema
            .as_object()
            .ok_or(McpConfigError::Invalid(
                "tool input schema must be an object",
            ))?;
        let mut tool = Tool::new(
            definition.name.clone(),
            definition.description.clone(),
            Arc::new(schema.clone()),
        )
        .with_annotations(tool_annotations(definition));
        if let Some(output) = &definition.output_schema {
            let schema = output.as_object().ok_or(McpConfigError::Invalid(
                "tool output schema must be an object",
            ))?;
            tool.output_schema = Some(Arc::new(schema.clone()));
        }
        protocol_tools.push(tool);
    }
    Ok(protocol_tools)
}

fn tool_annotations(definition: &ScopedTool) -> ToolAnnotations {
    let mut annotations = definition.annotations.clone();
    let read_only = *annotations
        .read_only_hint
        .get_or_insert(definition.read_only);
    annotations.destructive_hint.get_or_insert(!read_only);
    annotations.open_world_hint.get_or_insert(false);
    annotations
}

/// Exports protocol schemas and required scopes for drift checks.
pub fn tool_schema(definitions: &[ScopedTool]) -> Result<Value, McpConfigError> {
    Ok(
        json!({"tools": compile_tools(definitions)?, "scopes": definitions.iter().map(|tool| (&tool.name, &tool.required_scopes)).collect::<std::collections::BTreeMap<_, _>>()}),
    )
}

#[derive(Clone)]
pub(crate) struct ProductServer(pub Arc<RegisteredServices>);

impl ServerHandler for ProductServer {
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        let principal = principal(&context)?;
        let resources = self
            .0
            .resource_definitions
            .iter()
            .filter(|r| {
                self.0
                    .resource_scopes(&r.resource.uri)
                    .is_some_and(|scopes| permitted_scopes(principal, &scopes))
            })
            .map(|r| r.resource.clone())
            .collect();
        Ok(rmcp::model::ListResourcesResult::with_all_items(resources)
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        let principal = principal(&context)?;
        let templates = self
            .0
            .templates
            .iter()
            .filter(|r| permitted_scopes(principal, &r.required_scopes))
            .map(|r| r.template.clone())
            .collect();
        Ok(rmcp::model::ListResourceTemplatesResult::with_all_items(
            templates,
        ))
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResponse, ErrorData> {
        let principal = principal(&context)?;
        let service =
            self.0.resources.as_ref().ok_or_else(
                ErrorData::method_not_found::<rmcp::model::ReadResourceRequestMethod>,
            )?;
        let scopes = self
            .0
            .resource_scopes(&request.uri)
            .ok_or_else(|| ErrorData::invalid_params("Unknown resource", None))?;
        require_scopes(principal, &scopes)?;
        let contents = service
            .read(principal, &request.uri)
            .await
            .map_err(ErrorData::from)?;
        Ok(rmcp::model::ReadResourceResult::new(contents)
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private)
            .into())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        let principal = principal(&context)?;
        let prompts = self
            .0
            .prompt_definitions
            .iter()
            .filter(|p| permitted_scopes(principal, &p.required_scopes))
            .map(|p| p.prompt.clone())
            .collect();
        Ok(rmcp::model::ListPromptsResult::with_all_items(prompts)
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResponse, ErrorData> {
        let principal = principal(&context)?;
        let service = self
            .0
            .prompts
            .as_ref()
            .ok_or_else(ErrorData::method_not_found::<rmcp::model::GetPromptRequestMethod>)?;
        let definition = self
            .0
            .prompt_definitions
            .iter()
            .find(|p| p.prompt.name == request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown prompt", None))?;
        require_scopes(principal, &definition.required_scopes)?;
        let arguments = request.arguments.unwrap_or_default();
        let declared = definition.prompt.arguments.as_deref().unwrap_or_default();
        if arguments
            .iter()
            .any(|(name, value)| !value.is_string() || !declared.iter().any(|a| a.name == *name))
            || declared
                .iter()
                .any(|a| a.required == Some(true) && !arguments.contains_key(&a.name))
        {
            return Err(ErrorData::invalid_params("Invalid prompt arguments", None));
        }
        Ok(service
            .get(principal, &request.name, Value::Object(arguments))
            .await
            .map_err(ErrorData::from)?
            .into())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
            ProtocolVersion::V_2025_06_18,
        ])
    }

    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::builder().enable_tools().build();
        if self.0.resources.is_some() {
            capabilities.resources = Some(Default::default());
        }
        if self.0.prompts.is_some() {
            capabilities.prompts = Some(Default::default());
        }
        let mut config = ServerConfig::new(capabilities)
            .with_server_info(self.0.server_info.clone())
            .with_protocol_version(ProtocolVersion::V_2026_07_28);
        config.instructions = self.0.instructions.clone();
        config
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let principal = principal(&context)?;
        let tools = self
            .0
            .protocol_tools
            .iter()
            .zip(&self.0.definitions)
            .filter(|(_, definition)| permitted(principal, definition))
            .map(|(tool, _)| tool.clone())
            .collect();
        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.0
            .protocol_tools
            .iter()
            .find(|tool| tool.name == name)
            .cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let principal = principal(&context)?;
        let definition = self
            .0
            .tool(&request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown tool", None))?;
        if !permitted(principal, definition) {
            return Err(ErrorData::invalid_request("insufficient_scope", None));
        }
        let result = self
            .0
            .service
            .call(
                principal,
                &request.name,
                Value::Object(request.arguments.unwrap_or_default()),
            )
            .await;
        let response = match result {
            Ok(value) => {
                let mut result = CallToolResult::structured(value);
                if let Some(prefix) = &self.0.success_text_prefix {
                    for content in &mut result.content {
                        if let rmcp::model::ContentBlock::Text(text) = content {
                            text.text.insert_str(0, prefix);
                        }
                    }
                }
                result
            }
            Err(error) => error.into_result(),
        };
        Ok(response.into())
    }
}

fn principal(context: &RequestContext<RoleServer>) -> Result<&Principal, ErrorData> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<Principal>())
        .ok_or_else(|| ErrorData::invalid_request("Authentication required", None))
}

pub(crate) fn permitted(principal: &Principal, tool: &ScopedTool) -> bool {
    permitted_scopes(principal, &tool.required_scopes)
}

pub(crate) fn permitted_scopes(principal: &Principal, scopes: &[String]) -> bool {
    scopes
        .iter()
        .all(|scope| principal.scopes().contains(scope))
}

fn require_scopes(principal: &Principal, scopes: &[String]) -> Result<(), ErrorData> {
    if permitted_scopes(principal, scopes) {
        return Ok(());
    }
    Err(ErrorData::invalid_request("insufficient_scope", None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> ScopedTool {
        ScopedTool {
            name: "read_account".into(),
            description: "Read the account".into(),
            input_schema: json!({"type": "object", "additionalProperties": false}),
            output_schema: Some(json!({"type": "object", "required": ["id"]})),
            required_scopes: vec!["account:read".into()],
            read_only: true,
            annotations: Default::default(),
        }
    }

    #[tokio::test]
    async fn scope_filtering_requires_every_grant_from_the_effective_principal() {
        let issuer = baukit_test::MockOidcServer::start().await.expect("issuer");
        let config = baukit_auth::OidcConfig::new(issuer.issuer(), "https://mcp.example/mcp")
            .expect("config");
        let verifier = baukit_auth::OidcVerifier::discover(config)
            .await
            .expect("verifier");
        let claims = issuer
            .claims(
                "alice",
                "https://mcp.example/mcp",
                std::time::Duration::from_secs(300),
            )
            .expect("claims")
            .claim("scope", "items:read learning:read");
        let token = issuer.mint(&claims).expect("token");
        let principal = Principal::from(verifier.verify(&token).await.expect("principal"));
        assert!(permitted_scopes(
            &principal,
            &["items:read".into(), "learning:read".into()]
        ));
        assert!(!permitted_scopes(
            &principal,
            &["items:read".into(), "private:read".into()]
        ));
        let effective = principal.with_scopes(["items:read".into()]);
        assert!(permitted_scopes(&effective, &["items:read".into()]));
        assert!(!permitted_scopes(&effective, &["learning:read".into()]));
    }

    #[test]
    fn default_custom_and_text_only_tool_errors_preserve_their_content() {
        let default = ToolError::new("not_found", "Item not found").into_result();
        assert_eq!(default.is_error, Some(true));
        assert_eq!(
            default.structured_content,
            Some(json!({"code":"not_found","message":"Item not found"}))
        );
        let custom = ToolError::new("stale", "Reload revision 7")
            .with_structured_content(json!({"data":null,"error":{"currentRevision":7}}))
            .into_result();
        assert_eq!(custom.is_error, Some(true));
        assert_eq!(
            custom.structured_content,
            Some(json!({"data":null,"error":{"currentRevision":7}}))
        );
        assert_eq!(
            serde_json::to_value(custom.content).expect("content"),
            json!([{"type":"text","text":"Reload revision 7"}])
        );
        let text = ToolError::new("not_found", "Item not found")
            .text_only()
            .into_result();
        assert_eq!(text.is_error, Some(true));
        assert_eq!(text.structured_content, None);
        assert_eq!(
            serde_json::to_value(text.content).expect("content"),
            json!([{"type":"text","text":"Item not found"}])
        );
    }

    #[test]
    fn drift_contract_preserves_schemas_scopes_and_annotations() {
        let tool = definition();
        let contract = tool_schema(std::slice::from_ref(&tool)).expect("contract");
        assert_eq!(contract["tools"][0]["inputSchema"], tool.input_schema);
        assert_eq!(
            contract["tools"][0]["outputSchema"],
            tool.output_schema.expect("output")
        );
        assert_eq!(contract["scopes"]["read_account"], json!(["account:read"]));
        assert_eq!(contract["tools"][0]["annotations"]["readOnlyHint"], true);
        assert_eq!(
            contract["tools"][0]["annotations"]["destructiveHint"],
            false
        );
        assert_eq!(contract["tools"][0]["annotations"]["openWorldHint"], false);
    }

    #[test]
    fn annotations_merge_each_hint_with_the_read_write_defaults() {
        let read = definition();
        let mut write = read.clone();
        write.name = "create_account".into();
        write.read_only = false;
        let contract = tool_schema(&[read, write.clone()]).expect("contract");
        assert_eq!(
            contract["tools"][0]["annotations"],
            json!({
                "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false
            })
        );
        assert_eq!(
            contract["tools"][1]["annotations"],
            json!({
                "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false
            })
        );
        write.annotations = ToolAnnotations::new().open_world(true);
        let contract = tool_schema(&[write.clone()]).expect("contract");
        assert_eq!(
            contract["tools"][0]["annotations"],
            json!({
                "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true
            })
        );
        write.annotations = ToolAnnotations::with_title("Create account")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(true);
        let contract = tool_schema(&[write.clone()]).expect("contract");
        assert_eq!(
            contract["tools"][0]["annotations"],
            json!({
                "title": "Create account", "readOnlyHint": false, "destructiveHint": false,
                "idempotentHint": true, "openWorldHint": true
            })
        );
        write.annotations = ToolAnnotations::new().read_only(true).idempotent(false);
        let contract = tool_schema(&[write]).expect("contract");
        assert_eq!(
            contract["tools"][0]["annotations"],
            json!({
                "readOnlyHint": true, "destructiveHint": false,
                "idempotentHint": false, "openWorldHint": false
            })
        );
    }

    #[test]
    fn ambiguous_tools_invalid_schemas_and_unsafe_scopes_are_rejected() {
        let tool = definition();
        assert!(tool_schema(&[tool.clone(), tool.clone()]).is_err());
        for name in [String::new(), "x".repeat(MAX_TOOL_NAME_BYTES + 1)] {
            assert!(
                tool_schema(&[ScopedTool {
                    name,
                    ..tool.clone()
                }])
                .is_err()
            );
        }
        for scopes in [
            vec![],
            vec!["".into()],
            vec!["account:read other".into()],
            vec!["quote\"".into()],
            vec!["slash\\".into()],
        ] {
            assert!(
                tool_schema(&[ScopedTool {
                    required_scopes: scopes,
                    ..tool.clone()
                }])
                .is_err()
            );
        }
        assert!(
            tool_schema(&[ScopedTool {
                input_schema: json!([]),
                ..tool.clone()
            }])
            .is_err()
        );
        assert!(
            tool_schema(&[ScopedTool {
                output_schema: Some(json!(false)),
                ..tool
            }])
            .is_err()
        );
    }
}
