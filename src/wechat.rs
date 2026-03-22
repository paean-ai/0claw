use crate::agent::{self, AgentEvent};
use crate::config::LlmConfig;
use crate::mcp::McpManager;
use crate::store::Store;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

const CHANNEL_VERSION: &str = "0.1.0";
const MSG_TYPE_USER: i64 = 1;
const MSG_TYPE_BOT: i64 = 2;
const MSG_STATE_FINISH: i64 = 2;
const MSG_ITEM_TEXT: i64 = 1;
const MSG_ITEM_VOICE: i64 = 3;
const LONG_POLL_TIMEOUT_SECS: u64 = 35;
const SEND_TIMEOUT_SECS: u64 = 15;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const BACKOFF_DELAY_SECS: u64 = 30;
const RETRY_DELAY_SECS: u64 = 2;

#[derive(Deserialize, Clone)]
pub struct WechatConfig {
    pub token: String,
    pub base_url: String,
    pub account_id: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

#[derive(Deserialize)]
struct TextItem {
    text: Option<String>,
}

#[derive(Deserialize)]
struct VoiceItem {
    text: Option<String>,
}

#[derive(Deserialize)]
struct RefMsg {
    title: Option<String>,
}

#[derive(Deserialize)]
struct MessageItem {
    #[serde(rename = "type")]
    item_type: Option<i64>,
    text_item: Option<TextItem>,
    voice_item: Option<VoiceItem>,
    ref_msg: Option<RefMsg>,
}

#[derive(Deserialize)]
struct WeixinMessage {
    from_user_id: Option<String>,
    message_type: Option<i64>,
    item_list: Option<Vec<MessageItem>>,
    context_token: Option<String>,
}

#[derive(Deserialize)]
struct GetUpdatesResp {
    ret: Option<i64>,
    errcode: Option<i64>,
    msgs: Option<Vec<WeixinMessage>>,
    get_updates_buf: Option<String>,
}

fn extract_text(msg: &WeixinMessage) -> Option<String> {
    for item in msg.item_list.as_deref()? {
        if item.item_type == Some(MSG_ITEM_TEXT) {
            if let Some(ref ti) = item.text_item {
                if let Some(ref text) = ti.text {
                    if let Some(ref rm) = item.ref_msg {
                        if let Some(ref title) = rm.title {
                            return Some(format!("[Quote: {}]\n{}", title, text));
                        }
                    }
                    return Some(text.clone());
                }
            }
        }
        if item.item_type == Some(MSG_ITEM_VOICE) {
            if let Some(ref vi) = item.voice_item {
                if let Some(ref text) = vi.text {
                    return Some(text.clone());
                }
            }
        }
    }
    None
}

fn random_wechat_uin() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(t.as_nanos().to_string())
}

fn build_headers(token: Option<&str>) -> Vec<(String, String)> {
    let mut h = vec![
        ("Content-Type".into(), "application/json".into()),
        ("AuthorizationType".into(), "ilink_bot_token".into()),
        ("X-WECHAT-UIN".into(), random_wechat_uin()),
    ];
    if let Some(t) = token {
        if !t.trim().is_empty() {
            h.push(("Authorization".into(), format!("Bearer {}", t.trim())));
        }
    }
    h
}

fn norm(base: &str) -> String {
    if base.ends_with('/') { base.to_string() } else { format!("{}/", base) }
}

async fn get_updates(client: &reqwest::Client, base_url: &str, token: &str, buf: &str) -> Result<GetUpdatesResp, String> {
    let url = format!("{}ilink/bot/getupdates", norm(base_url));
    let body = json!({
        "get_updates_buf": buf,
        "base_info": { "channel_version": CHANNEL_VERSION },
    });
    let mut req = client.post(&url).timeout(std::time::Duration::from_secs(LONG_POLL_TIMEOUT_SECS)).json(&body);
    for (k, v) in build_headers(Some(token)) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

async fn send_text(client: &reqwest::Client, base_url: &str, token: &str, to: &str, text: &str, ctx_token: &str) -> Result<(), String> {
    let url = format!("{}ilink/bot/sendmessage", norm(base_url));
    let client_id = format!("0claw:{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
    let body = json!({
        "msg": {
            "from_user_id": "", "to_user_id": to, "client_id": client_id,
            "message_type": MSG_TYPE_BOT, "message_state": MSG_STATE_FINISH,
            "item_list": [{ "type": MSG_ITEM_TEXT, "text_item": { "text": text } }],
            "context_token": ctx_token,
        },
        "base_info": { "channel_version": CHANNEL_VERSION },
    });
    let mut req = client.post(&url).timeout(std::time::Duration::from_secs(SEND_TIMEOUT_SECS)).json(&body);
    for (k, v) in build_headers(Some(token)) {
        req = req.header(&k, &v);
    }
    req.send().await.map_err(|e| e.to_string())?;
    Ok(())
}

fn is_user_allowed(sender: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() { return true; }
    let name = sender.split('@').next().unwrap_or(sender);
    allowed.iter().any(|u| u == sender || u == name)
}

fn chunk_text(s: &str, max: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let end = (start + max).min(s.len());
        let end = if end < s.len() {
            s[start..end].rfind(char::is_whitespace).map_or(end, |p| start + p + 1)
        } else { end };
        chunks.push(&s[start..end]);
        start = end;
    }
    chunks
}

pub async fn start(wc: WechatConfig, llm: LlmConfig, system_prompt: String, store: Store, mcp: McpManager) {
    let client = reqwest::Client::new();
    let mut update_buf = String::new();
    let mut consecutive_failures: u32 = 0;
    let mut ctx_cache: HashMap<String, String> = HashMap::new();

    eprintln!("[0claw] 💬 WeChat channel connected (account: {})", wc.account_id);

    loop {
        match get_updates(&client, &wc.base_url, &wc.token, &update_buf).await {
            Err(e) => {
                consecutive_failures += 1;
                eprintln!("[0claw] WeChat poll error: {e}");
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    consecutive_failures = 0;
                    tokio::time::sleep(std::time::Duration::from_secs(BACKOFF_DELAY_SECS)).await;
                } else {
                    tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                }
                continue;
            }
            Ok(resp) => {
                let is_error = resp.ret.map_or(false, |r| r != 0) || resp.errcode.map_or(false, |e| e != 0);
                if is_error {
                    consecutive_failures += 1;
                    eprintln!("[0claw] WeChat getUpdates error: ret={:?} errcode={:?}", resp.ret, resp.errcode);
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        consecutive_failures = 0;
                        tokio::time::sleep(std::time::Duration::from_secs(BACKOFF_DELAY_SECS)).await;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                    }
                    continue;
                }
                consecutive_failures = 0;
                if let Some(buf) = resp.get_updates_buf {
                    update_buf = buf;
                }

                for msg in resp.msgs.unwrap_or_default() {
                    if msg.message_type != Some(MSG_TYPE_USER) { continue; }
                    let text = match extract_text(&msg) {
                        Some(t) if !t.is_empty() => t,
                        _ => continue,
                    };
                    let sender_id = msg.from_user_id.as_deref().unwrap_or("unknown");
                    if !is_user_allowed(sender_id, &wc.allowed_users) { continue; }
                    if let Some(ct) = &msg.context_token {
                        ctx_cache.insert(sender_id.to_string(), ct.clone());
                    }

                    let sender_name = sender_id.split('@').next().unwrap_or(sender_id);
                    let preview: String = if text.len() > 80 { format!("{}...", &text[..80]) } else { text.clone() };
                    eprintln!("[0claw] 💬 ← {sender_name}: {preview}");

                    let conv_id = format!("wechat-{sender_id}");
                    let title = format!("WeChat: {sender_name}");
                    let _ = store.create_conversation(&conv_id, &title);
                    let _ = store.add_message(&conv_id, "user", &text, None);

                    let hist = store.get_messages(&conv_id).unwrap_or_default();
                    let mut msgs: Vec<Value> = hist.iter().map(|m| {
                        let mut v = json!({ "role": &m.role, "content": &m.content });
                        if let Some(tc) = &m.tool_calls {
                            if let Ok(p) = serde_json::from_str::<Value>(tc) { v["tool_calls"] = p; }
                        }
                        v
                    }).collect();

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
                            AgentEvent::Content { text: t } => response.push_str(&t),
                            AgentEvent::ToolCall { name: ref n, .. } => {
                                eprintln!("[0claw]   🔧 {n}...");
                            }
                            AgentEvent::ToolResult { name: ref n, .. } => {
                                eprintln!("[0claw]   ✓ {n} done");
                            }
                            AgentEvent::Done { content } => { if response.is_empty() { response = content; } }
                            AgentEvent::Error { error: ref e } => {
                                eprintln!("[0claw]   ✗ Error: {e}");
                            }
                            AgentEvent::Start { .. } => {}
                        }
                    }

                    if !response.is_empty() {
                        let resp_preview: String = if response.len() > 100 { format!("{}...", &response[..100]) } else { response.clone() };
                        eprintln!("[0claw] 💬 → {sender_name}: {resp_preview}");

                        let _ = store.add_message(&conv_id, "assistant", &response, None);
                        if let Some(ctx) = ctx_cache.get(sender_id) {
                            for chunk in chunk_text(&response, 2048) {
                                if let Err(e) = send_text(&client, &wc.base_url, &wc.token, sender_id, chunk, ctx).await {
                                    eprintln!("[0claw] WeChat send error: {e}");
                                }
                            }
                            eprintln!("[0claw] ✓ Reply sent to {sender_name}");
                        }
                    }
                }
            }
        }
    }
}
