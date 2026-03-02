use crate::{
    agent::{self, AgentEvent},
    config::Config,
    mcp::McpManager,
    store::Store,
};
use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};

pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub mcp: McpManager,
    pub system_prompt: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/chat", post(chat))
        .route("/api/conversations", get(conversations))
        .route("/api/messages", get(messages))
        .with_state(state)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatReq {
    message: String,
    #[serde(default)]
    conversation_id: Option<String>,
}

async fn chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatReq>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let conv_id = req
        .conversation_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let title: String = req.message.chars().take(50).collect();
    let _ = state.store.create_conversation(&conv_id, &title);
    let _ = state.store.add_message(&conv_id, "user", &req.message, None);

    let (tx, rx) = mpsc::channel(64);
    let llm = state.config.llm.clone();
    let prompt = state.system_prompt.clone();
    let mcp = state.mcp.clone();
    let store = state.store.clone();
    let cid = conv_id.clone();

    tokio::spawn(async move {
        let _ = tx
            .send(AgentEvent::Start {
                conversation_id: cid.clone(),
            })
            .await;

        let hist = store.get_messages(&cid).unwrap_or_default();
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
        agent::run(&llm, &prompt, &mut msgs, &tools, &mcp, tx).await;

        if let Some(last) = msgs.last() {
            if last["role"].as_str() == Some("assistant") {
                let content = last["content"].as_str().unwrap_or("");
                let tc = last.get("tool_calls").map(|v| v.to_string());
                let _ = store.add_message(&cid, "assistant", content, tc.as_deref());
            }
        }
    });

    Sse::new(ReceiverStream::new(rx).map(|e| {
        Ok(Event::default().data(serde_json::to_string(&e).unwrap_or_default()))
    }))
}

async fn conversations(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(state.store.list_conversations().unwrap_or_default()))
}

#[derive(serde::Deserialize)]
struct MsgQuery {
    #[serde(rename = "conversationId")]
    conversation_id: String,
}

async fn messages(State(state): State<Arc<AppState>>, Query(q): Query<MsgQuery>) -> Json<Value> {
    Json(json!(state.store.get_messages(&q.conversation_id).unwrap_or_default()))
}
