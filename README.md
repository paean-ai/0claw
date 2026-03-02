# 0claw

> Defining the Absolute Core.

A minimal Rust agent runtime. ~500 lines. Zero overhead.

## Quick Start

```bash
cargo build --release
cp 0claw.toml.example 0claw.toml
# Edit 0claw.toml with your LLM API key
./target/release/0claw
```

## Architecture

```
src/main.rs    — entry point
src/config.rs  — TOML config + env interpolation
src/agent.rs   — LLM streaming + tool-calling loop
src/mcp.rs     — MCP stdio client
src/store.rs   — SQLite persistence
src/server.rs  — HTTP API (axum) + SSE
```

## API

| Method | Endpoint             | Description                    |
|--------|----------------------|--------------------------------|
| POST   | `/api/chat`          | Send message, returns SSE stream |
| GET    | `/api/conversations` | List conversations             |
| GET    | `/api/messages`      | Get messages by conversationId |

## Comparison

| Dimension | PaeanClaw | OpenClaw (ZeroClaw) | **0claw** |
|-----------|-----------|---------------------|-----------|
| Language | TypeScript | Rust | **Rust** |
| Lines of Code | ~365 | ~20,000+ | **~500** |
| Runtime | Node.js / Bun | Native binary | **Native binary** |
| Binary Size | N/A (interpreted) | ~10MB+ | **~5MB (release)** |
| Dependencies | 2 (npm) | 50+ crates | **10 crates** |
| LLM Provider | OpenAI-compatible | Multi-provider (OpenAI, Anthropic, Gemini, Ollama, etc.) | **OpenAI-compatible** |
| Tool System | MCP only | Built-in (50+) + MCP | **MCP only** |
| Channels | HTTP + Telegram | HTTP + 20+ channels (Telegram, Discord, Slack, etc.) | **HTTP only** |
| Storage | SQLite | SQLite + Vector memory | **SQLite** |
| Streaming | SSE | SSE + WebSocket | **SSE** |
| Config Format | JSON | TOML | **TOML** |
| Frontend | PWA (built-in) | Site (GitHub Pages) | **None (API only)** |
| Security | None | Pairing, E-stop, OTP, Sandbox | **None (local use)** |
| Hardware | None | ESP32, GPIO, probe-rs | **None** |
| Extension Model | Skills (Markdown) | Traits + Skills + Templates | **MCP servers** |
| Target | Local AI assistant | Full autonomous agent platform | **Minimal core runtime** |

### Design Philosophy

- **PaeanClaw**: Minimal local agent for personal use. Prioritizes simplicity and quick setup with Node.js/Bun.
- **OpenClaw (ZeroClaw)**: Comprehensive agent platform. Trait-driven architecture with extensive channel, provider, and tool support.
- **0claw**: The absolute core distilled. Pure Rust, single binary, zero bloat. Only the essential: LLM streaming, MCP tools, and conversation persistence.

## Configuration

```toml
[llm]
base_url = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY}"
model = "gpt-4o"

port = 3007

[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
```

Environment variables are interpolated via `${VAR}` syntax. Set `ZEROCLAW_CONFIG` to override the default config path (`0claw.toml`).

## Features

- Pure Rust, single binary
- Any OpenAI-compatible LLM
- MCP tool integration (JSON-RPC 2.0 over stdio)
- SSE streaming responses
- SQLite conversation persistence
- TOML config with env interpolation
- Release-optimized: `opt-level=z`, LTO, strip

## License

MIT
