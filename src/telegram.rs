use crate::agent::{self, AgentEvent};
use crate::config::{LlmConfig, TelegramConfig};
use crate::mcp::McpManager;
use crate::store::Store;
use serde_json::{json, Value};

pub async fn start(tg: TelegramConfig, llm: LlmConfig, system_prompt: String, store: Store, mcp: McpManager) {
    let client = reqwest::Client::new();
    let base = format!("https://api.telegram.org/bot{}", tg.token);

    let me: Value = match client.get(format!("{base}/getMe")).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => {
            eprintln!("[0claw] Telegram getMe failed: {e}");
            return;
        }
    };
    let bot_id = me["result"]["id"].as_i64().unwrap_or(0);
    let bot_username = me["result"]["username"].as_str().unwrap_or("").to_string();
    eprintln!("[0claw] Telegram bot @{bot_username} running");

    let mut offset: i64 = 0;
    loop {
        let resp: Value = match client
            .get(format!("{base}/getUpdates"))
            .query(&[("offset", offset.to_string()), ("timeout", "30".into())])
            .send()
            .await
        {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => {
                eprintln!("[0claw] Telegram poll error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        for update in resp["result"].as_array().cloned().unwrap_or_default() {
            offset = update["update_id"].as_i64().unwrap_or(0) + 1;

            let msg = &update["message"];
            let text = match msg["text"].as_str() {
                Some(t) => t,
                None => continue,
            };

            let chat_id = msg["chat"]["id"].as_i64().unwrap_or(0);
            let msg_id = msg["message_id"].as_i64().unwrap_or(0);
            let is_private = msg["chat"]["type"].as_str() == Some("private");
            let mentioned = text.contains(&format!("@{bot_username}"));
            let is_reply = msg["reply_to_message"]["from"]["id"].as_i64() == Some(bot_id);

            if !is_private && !mentioned && !is_reply {
                continue;
            }

            if !is_user_allowed(&msg["from"], &tg.allowed_users) {
                continue;
            }

            let user_msg = text.replace(&format!("@{bot_username}"), "").trim().to_string();
            if user_msg.is_empty() {
                continue;
            }

            let conv_id = format!("telegram-{chat_id}");
            let title = if is_private {
                format!("Telegram: {}", msg["from"]["first_name"].as_str().unwrap_or("DM"))
            } else {
                format!("Telegram: {}", msg["chat"]["title"].as_str().unwrap_or("Group"))
            };
            let _ = store.create_conversation(&conv_id, &title);
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

            let tools = mcp.list_tools().await.unwrap_or_default();
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let llm_c = llm.clone();
            let prompt_c = system_prompt.clone();
            let mcp_c = mcp.clone();

            tokio::spawn(async move {
                agent::run(&llm_c, &prompt_c, &mut msgs, &tools, &mcp_c, tx).await;
            });

            let mut response = String::new();
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::Content { text } => response.push_str(&text),
                    AgentEvent::Done { content } => {
                        if response.is_empty() {
                            response = content;
                        }
                    }
                    _ => {}
                }
            }

            if !response.is_empty() {
                let _ = store.add_message(&conv_id, "assistant", &response, None);
                for chunk in chunk_text(&response, 4096) {
                    let _ = client
                        .post(format!("{base}/sendMessage"))
                        .json(&json!({
                            "chat_id": chat_id,
                            "text": chunk,
                            "reply_to_message_id": msg_id,
                        }))
                        .send()
                        .await;
                }
            }
        }
    }
}

fn is_user_allowed(from: &Value, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let user_id = from["id"].as_i64().unwrap_or(0).to_string();
    let username = from["username"].as_str().unwrap_or("").to_lowercase();
    allowed.iter().any(|u| u == &user_id || (!username.is_empty() && u.to_lowercase() == username))
}

fn chunk_text(s: &str, max: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let end = (start + max).min(s.len());
        let end = if end < s.len() {
            s[start..end].rfind(char::is_whitespace).map_or(end, |p| start + p + 1)
        } else {
            end
        };
        chunks.push(&s[start..end]);
        start = end;
    }
    chunks
}
