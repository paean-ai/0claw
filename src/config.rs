use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone)]
pub struct Config {
    pub llm: LlmConfig,
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServer>,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Deserialize, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Deserialize, Clone)]
pub struct McpServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

fn default_port() -> u16 {
    3007
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let path = std::env::var("ZEROCLAW_CONFIG").unwrap_or_else(|_| "0claw.toml".into());
        let text = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&interpolate_env(&text))?)
    }
}

fn interpolate_env(input: &str) -> String {
    let mut result = input.to_string();
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let var = &result[start + 2..start + end];
            let val = std::env::var(var).unwrap_or_default();
            result.replace_range(start..start + end + 1, &val);
        } else {
            break;
        }
    }
    result
}
