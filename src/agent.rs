use crate::config::LlmConfig;
use crate::mcp::{McpManager, ToolSpec};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    #[serde(rename = "start")]
    Start { conversation_id: String },
    #[serde(rename = "content")]
    Content { text: String },
    #[serde(rename = "tool_call")]
    ToolCall { name: String, args: String },
    #[serde(rename = "tool_result")]
    ToolResult { name: String, result: String },
    #[serde(rename = "done")]
    Done { content: String },
    #[serde(rename = "error")]
    Error { error: String },
}

pub async fn run(
    config: &LlmConfig,
    system: &str,
    messages: &mut Vec<Value>,
    tools: &[ToolSpec],
    mcp: &McpManager,
    tx: mpsc::Sender<AgentEvent>,
) {
    let tool_defs: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": &t.name,
                    "description": &t.description,
                    "parameters": &t.parameters
                }
            })
        })
        .collect();

    let client = reqwest::Client::new();
    let mut full_content = String::new();

    for _ in 0..20 {
        let mut body = json!({
            "model": &config.model,
            "stream": true,
            "messages": std::iter::once(json!({ "role": "system", "content": system }))
                .chain(messages.iter().cloned())
                .collect::<Vec<Value>>(),
        });
        if !tool_defs.is_empty() {
            body["tools"] = json!(&tool_defs);
        }

        let resp = match client
            .post(format!("{}/chat/completions", config.base_url))
            .header("Authorization", format!("Bearer {}", config.api_key))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(AgentEvent::Error { error: e.to_string() }).await;
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let _ = tx
                .send(AgentEvent::Error {
                    error: format!("{status}: {body}"),
                })
                .await;
            return;
        }

        let mut resp = resp;
        let mut turn_content = String::new();
        let mut tc_buf: HashMap<usize, (String, String, String)> = HashMap::new();
        let mut buf = String::new();

        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].trim().to_string();
                        buf = buf[pos + 1..].to_string();
                        let Some(data) = line.strip_prefix("data: ") else {
                            continue;
                        };
                        if data == "[DONE]" {
                            continue;
                        }
                        let Ok(v) = serde_json::from_str::<Value>(data) else {
                            continue;
                        };
                        let delta = &v["choices"][0]["delta"];
                        if let Some(c) = delta["content"].as_str() {
                            turn_content.push_str(c);
                            let _ = tx.send(AgentEvent::Content { text: c.into() }).await;
                        }
                        if let Some(tcs) = delta["tool_calls"].as_array() {
                            for tc in tcs {
                                let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                let e = tc_buf.entry(idx).or_insert_with(|| {
                                    (
                                        tc["id"].as_str().unwrap_or("").into(),
                                        tc["function"]["name"].as_str().unwrap_or("").into(),
                                        String::new(),
                                    )
                                });
                                if let Some(n) = tc["function"]["name"].as_str() {
                                    if !n.is_empty() {
                                        e.1 = n.into();
                                    }
                                }
                                if let Some(a) = tc["function"]["arguments"].as_str() {
                                    e.2.push_str(a);
                                }
                            }
                        }
                    }
                }
                _ => break,
            }
        }

        full_content.push_str(&turn_content);

        if tc_buf.is_empty() {
            if !turn_content.is_empty() {
                messages.push(json!({ "role": "assistant", "content": &turn_content }));
            }
            break;
        }

        let mut sorted: Vec<(usize, (String, String, String))> = tc_buf.into_iter().collect();
        sorted.sort_by_key(|(k, _)| *k);

        let tool_calls: Vec<Value> = sorted
            .iter()
            .map(|(_, (id, name, args))| {
                json!({ "id": id, "type": "function", "function": { "name": name, "arguments": args } })
            })
            .collect();

        let mut amsg = json!({ "role": "assistant", "tool_calls": &tool_calls });
        if !turn_content.is_empty() {
            amsg["content"] = json!(&turn_content);
        }
        messages.push(amsg);

        for tc in &tool_calls {
            let name = tc["function"]["name"].as_str().unwrap_or("");
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let tc_id = tc["id"].as_str().unwrap_or("");

            let _ = tx
                .send(AgentEvent::ToolCall {
                    name: name.into(),
                    args: args_str.into(),
                })
                .await;

            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            let result = mcp
                .call_tool(name, args)
                .await
                .unwrap_or_else(|e| format!("Error: {e}"));

            let _ = tx
                .send(AgentEvent::ToolResult {
                    name: name.into(),
                    result: result.clone(),
                })
                .await;

            messages.push(json!({ "role": "tool", "tool_call_id": tc_id, "content": &result }));
        }
    }

    let _ = tx.send(AgentEvent::Done { content: full_content }).await;
}
