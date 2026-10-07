//! OAuth-protected MCP over stateless Streamable HTTP.

mod config;
mod introspection;
mod policy;
mod security;
mod server;

pub use config::{McpConfig, McpConfigError, ProtectedResourceMetadata};
pub use introspection::{KeycloakIntrospectionConfig, KeycloakIntrospectionPolicy};
pub use policy::{
    AuthenticationPolicy, JwtOnlyPolicy, PolicyDenial, PolicyFuture, Principal, VerifiedPrincipal,
};
pub use security::router;
pub use server::{ScopedTool, ToolError, ToolFuture, ToolService, tool_schema};

/// MCP request headers to add to a product's CORS policy.
pub const ALLOWED_HEADERS: [&str; 3] = ["mcp-protocol-version", "mcp-method", "mcp-name"];
