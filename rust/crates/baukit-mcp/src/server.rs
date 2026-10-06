use std::{borrow::Cow, collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use baukit_auth::Principal;
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

use crate::McpConfigError;

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
}

pub(crate) struct RegisteredTools {
    pub service: Arc<dyn ToolService>,
    pub definitions: Vec<ScopedTool>,
    pub protocol_tools: Vec<Tool>,
}

impl RegisteredTools {
    pub fn new(service: Arc<dyn ToolService>) -> Result<Self, McpConfigError> {
        let definitions = service.tools();
        let protocol_tools = compile_tools(&definitions)?;
        Ok(Self {
            service,
            definitions,
            protocol_tools,
        })
    }

    pub fn scopes(&self) -> Vec<String> {
        self.definitions
            .iter()
            .flat_map(|tool| tool.required_scopes.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn tool(&self, name: &str) -> Option<&ScopedTool> {
        self.definitions.iter().find(|tool| tool.name == name)
    }
}

fn valid_scope(scope: &str) -> bool {
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
        .with_annotations(
            ToolAnnotations::new()
                .read_only(definition.read_only)
                .destructive(!definition.read_only)
                .open_world(false),
        );
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

/// Exports protocol schemas and required scopes for drift checks.
pub fn tool_schema(definitions: &[ScopedTool]) -> Result<Value, McpConfigError> {
    Ok(
        json!({"tools": compile_tools(definitions)?, "scopes": definitions.iter().map(|tool| (&tool.name, &tool.required_scopes)).collect::<std::collections::BTreeMap<_, _>>()}),
    )
}

#[derive(Clone)]
pub(crate) struct ProductServer(pub Arc<RegisteredTools>);

impl ServerHandler for ProductServer {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
            ProtocolVersion::V_2025_06_18,
        ])
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("baukit-mcp", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
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
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(
                json!({"code": error.code, "message": error.message}),
            ),
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
    tool.required_scopes
        .iter()
        .all(|scope| principal.scopes().contains(scope))
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
        }
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
