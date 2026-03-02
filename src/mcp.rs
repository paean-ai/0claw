use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

struct McpProcess {
    _child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

#[derive(Clone)]
pub struct McpManager {
    servers: Arc<Mutex<HashMap<String, McpProcess>>>,
}

#[derive(Clone, serde::Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn connect(&self, name: &str, command: &str, args: &[String]) -> Result<()> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let mut proc = McpProcess {
            _child: child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
        };

        let init = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "0claw", "version": "0.1.0" }
            }
        });
        Self::rpc(&mut proc, &init).await?;

        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        proc.stdin
            .write_all(format!("{notif}\n").as_bytes())
            .await?;
        proc.stdin.flush().await?;

        self.servers.lock().await.insert(name.to_string(), proc);
        Ok(())
    }

    async fn rpc(proc: &mut McpProcess, request: &Value) -> Result<Value> {
        proc.stdin
            .write_all(format!("{request}\n").as_bytes())
            .await?;
        proc.stdin.flush().await?;
        let mut line = String::new();
        proc.reader.read_line(&mut line).await?;
        Ok(serde_json::from_str(line.trim())?)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolSpec>> {
        let mut tools = Vec::new();
        let mut servers = self.servers.lock().await;
        for (name, proc) in servers.iter_mut() {
            let id = proc.next_id;
            proc.next_id += 1;
            let req = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {} });
            let resp = match Self::rpc(proc, &req).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            if let Some(arr) = resp["result"]["tools"].as_array() {
                for t in arr {
                    tools.push(ToolSpec {
                        name: format!("{}__{}", name, t["name"].as_str().unwrap_or("")),
                        description: t["description"].as_str().unwrap_or("").into(),
                        parameters: t["inputSchema"].clone(),
                    });
                }
            }
        }
        Ok(tools)
    }

    pub async fn call_tool(&self, full_name: &str, args: Value) -> Result<String> {
        let (server, tool) = full_name
            .split_once("__")
            .ok_or_else(|| anyhow::anyhow!("invalid tool name: {full_name}"))?;
        let mut servers = self.servers.lock().await;
        let proc = servers
            .get_mut(server)
            .ok_or_else(|| anyhow::anyhow!("unknown server: {server}"))?;
        let id = proc.next_id;
        proc.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": args }
        });
        let resp = Self::rpc(proc, &req).await?;
        if let Some(content) = resp["result"]["content"].as_array() {
            Ok(content
                .iter()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"))
        } else if let Some(err) = resp["error"]["message"].as_str() {
            Ok(format!("Error: {err}"))
        } else {
            Ok(resp.to_string())
        }
    }
}
