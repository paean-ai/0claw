use crate::agent::{self, AgentEvent};
use crate::config::LlmConfig;
use crate::loop_sched::{self, LoopPromptEvent};
use crate::mcp::McpManager;
use crate::store::Store;
use serde_json::{json, Value};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

static AGENT_BUSY: AtomicBool = AtomicBool::new(false);

pub async fn start(
    llm: LlmConfig,
    system_prompt: String,
    store: Store,
    mcp: McpManager,
) {
    let conv_id = format!("cli-{}", uuid::Uuid::new_v4());
    let _ = store.create_conversation(&conv_id, "CLI session");

    let tools = mcp.list_tools().await.unwrap_or_default();
    let loop_tools = loop_sched::tool_definitions();
    let all_tools: Vec<_> = tools.iter().chain(loop_tools.iter()).cloned().collect();

    let mut loop_rx = loop_sched::take_receiver();

    eprintln!("[0claw] Interactive mode. Type /exit to quit.\n");

    loop {
        // Print prompt
        eprint!("\x1b[36myou>\x1b[0m ");
        io::stderr().flush().ok();

        // Select between user input and loop events
        let user_msg = tokio::select! {
            line = tokio::task::spawn_blocking(|| {
                let mut line = String::new();
                match io::stdin().read_line(&mut line) {
                    Ok(0) => None,
                    Ok(_) => Some(line.trim().to_string()),
                    Err(_) => None,
                }
            }) => {
                match line {
                    Ok(Some(l)) => l,
                    _ => break,
                }
            }
            Some(event) = recv_loop(&mut loop_rx) => {
                eprintln!("\n\x1b[33m[loop: {}] {}\x1b[0m", event.schedule, event.prompt);
                event.prompt
            }
        };

        if user_msg.is_empty() {
            continue;
        }
        if user_msg == "/exit" || user_msg == "/quit" {
            break;
        }

        let _ = store.add_message(&conv_id, "user", &user_msg, None);

        let hist = store.get_messages(&conv_id).unwrap_or_default();
        let mut msgs: Vec<Value> = hist
            .iter()
            .map(|m| {
                let mut v = json!({ "role": &m.role, "content": &m.content });
                if let Some(tc) = &m.tool_calls {
                    if let Ok(p) = serde_json::from_str::<Value>(tc) {
                        v["tool_calls"] = p;
                    }
                }
                v
            })
            .collect();

        let (tx, mut rx) = mpsc::channel(64);

        let llm_c = llm.clone();
        let prompt_c = system_prompt.clone();
        let mcp_c = mcp.clone();
        let all_tools_c = all_tools.clone();

        AGENT_BUSY.store(true, Ordering::SeqCst);

        tokio::spawn(async move {
            agent::run(&llm_c, &prompt_c, &mut msgs, &all_tools_c, &mcp_c, tx).await;
        });

        eprint!("\x1b[32m0claw>\x1b[0m ");
        io::stderr().flush().ok();

        let mut full_response = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::Content { text } => {
                    print!("{text}");
                    io::stdout().flush().ok();
                    full_response.push_str(&text);
                }
                AgentEvent::ToolCall { name, args } => {
                    // Handle built-in loop tools
                    if name.starts_with("loop_") {
                        let result = loop_sched::handle_tool_call(&name, &args).await;
                        eprintln!("\n  \x1b[90m[{name}] {result}\x1b[0m");
                    } else {
                        eprintln!("\n  \x1b[90m[calling {name}]\x1b[0m");
                    }
                }
                AgentEvent::ToolResult { name, result } => {
                    if !name.starts_with("loop_") {
                        let preview: String = result.chars().take(200).collect();
                        eprintln!("  \x1b[90m[{name} → {preview}]\x1b[0m");
                    }
                }
                AgentEvent::Done { content } => {
                    if full_response.is_empty() && !content.is_empty() {
                        print!("{content}");
                    }
                    println!();
                }
                AgentEvent::Error { error } => {
                    eprintln!("\n\x1b[31merror: {error}\x1b[0m");
                }
                _ => {}
            }
        }

        AGENT_BUSY.store(false, Ordering::SeqCst);

        // Save assistant response
        if !full_response.is_empty() {
            let _ = store.add_message(&conv_id, "assistant", &full_response, None);
        }
    }

    eprintln!("[0claw] bye.");
}

pub fn is_agent_busy() -> bool {
    AGENT_BUSY.load(Ordering::SeqCst)
}

async fn recv_loop(rx: &mut Option<mpsc::UnboundedReceiver<LoopPromptEvent>>) -> Option<LoopPromptEvent> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}
