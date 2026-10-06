//! OAuth-protected MCP over stateless Streamable HTTP.

mod config;
mod security;
mod server;

pub use baukit_auth::Principal;
pub use config::{McpConfig, McpConfigError, ProtectedResourceMetadata};
pub use security::router;
pub use server::{ScopedTool, ToolError, ToolFuture, ToolService, tool_schema};

/// MCP request headers to add to a product's CORS policy.
pub const ALLOWED_HEADERS: [&str; 3] = ["mcp-protocol-version", "mcp-method", "mcp-name"];
