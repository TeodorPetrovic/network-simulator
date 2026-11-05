# MCP server and AI copilot

Both features reuse the controller's own REST API, so authentication and RBAC apply unchanged.

## MCP server

`POST /v3/mcp` speaks MCP over Streamable HTTP (JSON responses, protocol 2025-03-26). Send the
same `Authorization: Bearer <token>` header as for any API call; with auth disabled no header is needed.

Each tool is one entry in `src/api/mcp_tools.rs` (name, REST method, path template, parameters).
Path placeholders are filled from arguments; other arguments become the query (GET/DELETE) or the
JSON body (POST/PUT). A `data` object is merged into the body. Tools run as loopback requests that
carry the caller's token, so a user can never do through MCP what the REST API would refuse.

Covered: projects, nodes (create from template, lifecycle, isolate, duplicate), links (create,
filters, capture), templates, snapshots, drawings, computes, and `node_run_command`, which runs one
command on a started QEMU device's serial console (login handled) and returns the output.

Example client config (Claude Code): `claude mcp add --transport http simulator http://HOST:3080/v3/mcp --header "Authorization: Bearer <token>"`

Not implemented: the legacy SSE transport and server-initiated notifications (`GET /v3/mcp` returns 405).

## AI copilot

Tools > AI copilot… opens a floating chat window. Each user adds their own model:

| Provider | Notes |
|---|---|
| Anthropic | Messages API; only the API key is needed |
| OpenAI-compatible | OpenAI, OpenRouter, DeepSeek, Gemini's OpenAI endpoint, LM Studio… set the base URL |
| Ollama | local server, no key |

API keys are stored in `llm_model_configs` and are never returned by the API (`has_api_key` only).
Chats are stored per user and project in `copilot_sessions`. The agent loop (`src/api/copilot.rs`)
runs up to 14 model/tool rounds per message and streams `session`, `text`, `tool_call`,
`tool_result`, `error` and `done` events as SSE. The "Read-only" switch limits the model to GET tools.

Endpoints: `/v3/copilot/llm-configs[/{id}]`, `/v3/copilot/projects/{id}/chat/{stream,sessions[/{sid}]}`.

Not implemented from GNS3 3.1: group-shared model configs, skills repositories, netmiko device
types, LangGraph-style checkpoints, chat abort/inject endpoints (the UI aborts by closing the stream).
