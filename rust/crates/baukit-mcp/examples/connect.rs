//! Connect to a generated remote MCP server with an already-issued access token.
use std::{env, error::Error};

use rmcp::{
    ClientServiceExt,
    model::{CallToolRequestParams, ProtocolVersion},
    service::ClientLifecycleMode,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let resource = env::var("MCP_RESOURCE_URL")?;
    let token = env::var("MCP_ACCESS_TOKEN")?;
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::builder().no_proxy().build()?,
        StreamableHttpClientTransportConfig::with_uri(resource).auth_header(token),
    );
    let client = ()
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await?;
    println!(
        "server/discover: {}",
        serde_json::to_string(&client.peer_info())?
    );
    let tools = client.list_all_tools().await?;
    if !tools.iter().any(|tool| tool.name == "list_items") {
        return Err("list_items is not registered".into());
    }
    println!("tools/list: {}", serde_json::to_string(&tools)?);
    let result = client
        .call_tool(CallToolRequestParams::new("list_items"))
        .await?;
    println!("tools/call list_items: {}", serde_json::to_string(&result)?);
    if result.is_error != Some(false) {
        return Err("list_items returned a tool error".into());
    }
    let items = result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("items"))
        .and_then(serde_json::Value::as_array)
        .ok_or("list_items omitted its items array")?;
    if let Ok(expected) = env::var("MCP_EXPECT_ITEM_NAME")
        && !items.iter().any(|item| item["name"] == expected)
    {
        return Err("list_items did not return the seeded item".into());
    }
    client.cancel().await?;
    Ok(())
}
