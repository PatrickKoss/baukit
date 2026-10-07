//! OAuth-protected MCP over stateless Streamable HTTP.

mod cancellation;
mod capabilities;
mod config;
mod introspection;
mod policy;
mod security;
mod server;

pub use capabilities::{
    CapabilityError, CapabilityFuture, McpServices, Prompt, PromptArgument, PromptFuture,
    PromptMessage, PromptResult, PromptService, Resource, ResourceContents, ResourceFuture,
    ResourceService, ResourceTemplate, Role, ScopedPrompt, ScopedResource, ScopedResourceTemplate,
    capability_schema, prompt_schema, resource_schema, service_schema,
};
pub use rmcp::model::{Implementation, ToolAnnotations};
pub use tokio_util::sync::CancellationToken;

pub use config::{McpConfig, McpConfigError, ProtectedResourceMetadata};
pub use introspection::{KeycloakIntrospectionConfig, KeycloakIntrospectionPolicy};
pub use policy::{
    AuthenticationPolicy, JwtOnlyPolicy, PolicyDenial, PolicyFuture, Principal, VerifiedPrincipal,
};
pub use security::router;
pub use server::{ScopedTool, ToolError, ToolFuture, ToolService, tool_schema};

/// MCP request headers to add to a product's CORS policy.
pub const ALLOWED_HEADERS: [&str; 3] = ["mcp-protocol-version", "mcp-method", "mcp-name"];
