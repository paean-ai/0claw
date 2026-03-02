mod agent;
mod config;
mod mcp;
mod server;
mod store;

use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::Config::load()?;
    let store = store::Store::new()?;
    let mcp = mcp::McpManager::new();

    for (name, srv) in &config.mcp_servers {
        if let Err(e) = mcp.connect(name, &srv.command, &srv.args).await {
            eprintln!("[0claw] MCP '{name}' failed: {e}");
        }
    }

    let system_prompt = std::fs::read_to_string("AGENT.md")
        .unwrap_or_else(|_| "You are a helpful assistant.".into());

    let port = config.port;
    let state = Arc::new(server::AppState {
        config,
        store,
        mcp,
        system_prompt,
    });

    eprintln!("[0claw] http://localhost:{port}");
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, server::router(state)).await?;
    Ok(())
}
