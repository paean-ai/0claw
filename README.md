# 0claw

> Defining the Absolute Core.

A minimal Rust agent runtime. ~500 lines. Zero overhead.

## Install

```bash
# One-line install (macOS / Linux) — re-run to update
curl -fsSL https://0.works/install.sh | bash

# Or install via Cargo
cargo install zero-claw
```

## Quick Start

```bash
cp 0claw.toml.example 0claw.toml
# Edit 0claw.toml with your LLM API key
0claw
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

### SSE Events

```
data: {"type":"start","conversation_id":"..."}
data: {"type":"content","text":"Hello"}
data: {"type":"tool_call","name":"fs__read_file","args":"..."}
data: {"type":"tool_result","name":"fs__read_file","result":"..."}
data: {"type":"done","content":"Hello world"}
```

## Comparison

| Dimension | PaeanClaw | [OpenClaw](https://github.com/openclaw/openclaw) | **0claw** |
|-----------|-----------|-----------|-----------|
| Language | TypeScript | TypeScript (85.9%) + Swift + Kotlin | **Rust** |
| Codebase | ~365 lines | Large monorepo (Gateway + CLI + UI + native apps) | **~500 lines** |
| Runtime | Node.js / Bun | Node.js ≥22 (pnpm / bun) | **Native binary** |
| Binary Size | N/A (interpreted) | N/A (interpreted + native apps) | **~5MB (release)** |
| Dependencies | 2 (npm) | Heavy (npm + native toolchains) | **10 crates** |
| LLM Provider | OpenAI-compatible | Multi-provider with model failover (OpenAI, Anthropic, etc.) | **OpenAI-compatible** |
| Tool System | MCP only | Built-in (browser, canvas, cron, nodes) + Skills | **MCP only** |
| Channels | HTTP + Telegram | 22+ channels (WhatsApp, Telegram, Slack, Discord, Signal, iMessage, Teams, Matrix, IRC, etc.) | **HTTP only** |
| Storage | SQLite | Sessions + workspace persistence | **SQLite** |
| Streaming | SSE | WebSocket (Gateway control plane) + SSE | **SSE** |
| Config Format | JSON | JSON (openclaw.json) | **TOML** |
| Frontend | PWA (built-in) | Control UI + WebChat (Gateway-served) + Canvas/A2UI | **None (API only)** |
| Native Apps | None | macOS menu bar + iOS + Android | **None** |
| Voice | None | Voice Wake + Talk Mode (macOS/iOS/Android) | **None** |
| Security | None | DM pairing, Docker sandbox, Tailscale, per-session isolation | **None (local use)** |
| Extension Model | Skills (Markdown) | Skills platform (bundled/managed/workspace) + ClawHub | **MCP servers** |
| Target | Local AI assistant | Full personal AI assistant platform | **Minimal core runtime** |

### Design Philosophy

- **PaeanClaw**: Minimal local agent for personal use. Prioritizes simplicity and quick setup with Node.js/Bun.
- **[OpenClaw](https://github.com/openclaw/openclaw)**: Comprehensive personal AI assistant. Gateway-centric architecture with 22+ messaging channels, native apps, voice, canvas, browser control, and Docker sandboxing. "Your own personal AI assistant. Any OS. Any Platform."
- **0claw**: The absolute core distilled. Pure Rust, single binary, zero bloat. Only the essential: LLM streaming, MCP tools, and conversation persistence.

## Configuration

```toml
# Top-level settings must come before table sections
port = 3007

[llm]
base_url = "https://api.paean.ai/v1"
api_key = "${PAEAN_API_KEY}"
model = "GLM-4.5"

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
