use crate::agent::AgentEvent;
use crate::config::LlmConfig;
use crate::mcp::{McpManager, ToolSpec};
use serde_json::{json, Value};
use tokio::sync::mpsc;

/// Run agent loop using Anthropic Messages API protocol.
/// Compatible with z.ai (open.bigmodel.cn/api/anthropic) and native Anthropic API.
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
                "name": &t.name,
                "description": &t.description,
                "input_schema": &t.parameters
            })
        })
        .collect();

    let client = reqwest::Client::new();
    let mut full_content = String::new();

    for _ in 0..20 {
        // Convert messages to Anthropic format (no system role in messages)
        let api_messages = to_anthropic_messages(messages);

        let mut body = json!({
            "model": &config.model,
            "max_tokens": 16384,
            "stream": true,
            "system": system,
            "messages": api_messages,
        });
        if !tool_defs.is_empty() {
            body["tools"] = json!(&tool_defs);
        }

        let resp = match client
            .post(format!("{}/v1/messages", config.base_url))
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx
                    .send(AgentEvent::Error {
                        error: e.to_string(),
                    })
                    .await;
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
        let mut tool_uses: Vec<(String, String, String)> = Vec::new(); // (id, name, input_json)
        let mut current_tool_input = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut buf = String::new();
        let mut stop_reason = String::new();

        while let Ok(Some(chunk)) = resp.chunk().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf = buf[pos + 1..].to_string();
                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(data) else {
                    continue;
                };

                let event_type = v["type"].as_str().unwrap_or("");

                match event_type {
                    "content_block_start" => {
                        let block = &v["content_block"];
                        if block["type"].as_str() == Some("tool_use") {
                            current_tool_id =
                                block["id"].as_str().unwrap_or("").to_string();
                            current_tool_name =
                                block["name"].as_str().unwrap_or("").to_string();
                            current_tool_input.clear();
                        }
                    }
                    "content_block_delta" => {
                        let delta = &v["delta"];
                        match delta["type"].as_str().unwrap_or("") {
                            "text_delta" => {
                                if let Some(text) = delta["text"].as_str() {
                                    turn_content.push_str(text);
                                    let _ = tx
                                        .send(AgentEvent::Content { text: text.into() })
                                        .await;
                                }
                            }
                            "input_json_delta" => {
                                if let Some(partial) = delta["partial_json"].as_str() {
                                    current_tool_input.push_str(partial);
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if !current_tool_id.is_empty() {
                            tool_uses.push((
                                current_tool_id.clone(),
                                current_tool_name.clone(),
                                current_tool_input.clone(),
                            ));
                            current_tool_id.clear();
                            current_tool_name.clear();
                            current_tool_input.clear();
                        }
                    }
                    "message_delta" => {
                        if let Some(sr) = v["delta"]["stop_reason"].as_str() {
                            stop_reason = sr.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }

        full_content.push_str(&turn_content);

        if tool_uses.is_empty() {
            if !turn_content.is_empty() {
                messages.push(json!({
                    "role": "assistant",
                    "content": turn_content,
                    "_anthropic_content": [{"type": "text", "text": &turn_content}]
                }));
            }
            break;
        }

        // Build assistant message with content blocks (text + tool_use)
        let mut content_blocks: Vec<Value> = Vec::new();
        if !turn_content.is_empty() {
            content_blocks.push(json!({"type": "text", "text": &turn_content}));
        }
        for (id, name, input_str) in &tool_uses {
            let input: Value = serde_json::from_str(input_str).unwrap_or(json!({}));
            content_blocks.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
        messages.push(json!({
            "role": "assistant",
            "content": "",
            "_anthropic_content": &content_blocks
        }));

        // Execute tools and collect results
        let mut tool_results: Vec<Value> = Vec::new();
        for (id, name, input_str) in &tool_uses {
            let _ = tx
                .send(AgentEvent::ToolCall {
                    name: name.clone(),
                    args: input_str.clone(),
                })
                .await;

            let result = crate::agent::call_tool(mcp, name, input_str).await;

            let _ = tx
                .send(AgentEvent::ToolResult {
                    name: name.clone(),
                    result: result.clone(),
                })
                .await;

            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": result
            }));
        }

        messages.push(json!({
            "role": "user",
            "content": "",
            "_anthropic_content": &tool_results
        }));

        if stop_reason != "tool_use" {
            break;
        }
    }

    let _ = tx
        .send(AgentEvent::Done {
            content: full_content,
        })
        .await;
}

/// Convert internal messages to Anthropic API format.
/// Uses `_anthropic_content` field if present (for tool_use/tool_result blocks),
/// otherwise converts from the OpenAI-style format.
fn to_anthropic_messages(messages: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");

        // If we have native anthropic content blocks stored, use them
        if let Some(blocks) = msg.get("_anthropic_content") {
            if blocks.is_array() {
                result.push(json!({
                    "role": role,
                    "content": blocks
                }));
                continue;
            }
        }

        // Skip system messages (handled as top-level param)
        if role == "system" {
            continue;
        }

        // Convert tool role messages (OpenAI format from store) to user with tool_result
        if role == "tool" {
            let tool_call_id = msg["tool_call_id"].as_str().unwrap_or("");
            let content = msg["content"].as_str().unwrap_or("");
            result.push(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content
                }]
            }));
            continue;
        }

        // Convert assistant messages with tool_calls (OpenAI format) to anthropic
        if role == "assistant" {
            if let Some(tool_calls) = msg["tool_calls"].as_array() {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(c) = msg["content"].as_str() {
                    if !c.is_empty() {
                        blocks.push(json!({"type": "text", "text": c}));
                    }
                }
                for tc in tool_calls {
                    let input: Value =
                        serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))
                            .unwrap_or(json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc["id"],
                        "name": tc["function"]["name"],
                        "input": input
                    }));
                }
                result.push(json!({"role": "assistant", "content": blocks}));
                continue;
            }
        }

        // Simple text message
        let content = msg["content"].as_str().unwrap_or("");
        result.push(json!({
            "role": role,
            "content": content
        }));
    }

    result
}
