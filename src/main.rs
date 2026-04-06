mod agent;
mod anthropic;
mod cli;
mod config;
mod loop_sched;
mod mcp;
mod server;
mod store;
mod telegram;
mod wechat;

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

    // Check for --cli flag
    let is_cli = std::env::args().any(|a| a == "--cli" || a == "-i");

    if is_cli {
        cli::start(config.llm.clone(), system_prompt, store, mcp).await;
        return Ok(());
    }

    let port = config.port;
    let tg_config = config.telegram.clone();
    let tg_llm = config.llm.clone();
    let tg_prompt = system_prompt.clone();
    let tg_store = store.clone();
    let tg_mcp = mcp.clone();

    let wc_config = config.wechat.clone();
    let wc_llm = config.llm.clone();
    let wc_prompt = system_prompt.clone();
    let wc_store = store.clone();
    let wc_mcp = mcp.clone();

    let has_telegram = tg_config.is_some();
    let has_wechat = wc_config.is_some();

    let state = Arc::new(server::AppState {
        config,
        store,
        mcp,
        system_prompt,
    });

    if let Some(tg) = tg_config {
        tokio::spawn(async move {
            telegram::start(tg, tg_llm, tg_prompt, tg_store, tg_mcp).await;
        });
    }

    if let Some(wc) = wc_config {
        tokio::spawn(async move {
            wechat::start(wc, wc_llm, wc_prompt, wc_store, wc_mcp).await;
        });
    }

    eprintln!("[0claw] http://localhost:{port}");
    if has_telegram { eprintln!("[0claw] Telegram channel: active"); }
    if has_wechat { eprintln!("[0claw] WeChat channel: active"); }
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, server::router(state)).await?;
    Ok(())
}
