//! Model Context Protocol server (Streamable HTTP, JSON responses) at `/v3/mcp`.
//!
//! Tools run as loopback REST calls carrying the caller's `Authorization` header, so the
//! existing authentication and RBAC rules apply to every tool unchanged.

use super::mcp_tools::{TOOLS, Tool, input_schema};
use super::*;

const PROTOCOL_VERSION: &str = "2025-03-26";
const MAX_RESULT_BYTES: usize = 200_000;

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

pub(super) async fn mcp_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(message): Json<Value>,
) -> Response {
    if let Some(batch) = message.as_array() {
        let mut replies = Vec::new();
        for item in batch {
            if let Some(reply) = handle_message(&state, &headers, item).await {
                replies.push(reply);
            }
        }
        return if replies.is_empty() {
            StatusCode::ACCEPTED.into_response()
        } else {
            Json(Value::Array(replies)).into_response()
        };
    }
    match handle_message(&state, &headers, &message).await {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

pub(super) async fn mcp_get() -> Response {
    // No server-initiated stream is offered; clients fall back to POST-only operation.
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

pub(super) async fn mcp_delete() -> Response {
    StatusCode::OK.into_response()
}

async fn handle_message(state: &AppState, headers: &HeaderMap, message: &Value) -> Option<Value> {
    let method = message["method"].as_str()?;
    // Notifications carry no id and get no reply.
    let id = message.get("id").cloned()?;
    Some(match method {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "network-simulator", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Network simulator control. List projects, open one, create nodes from templates, link them, start them, then use node_run_command to inspect devices.",
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(
            id,
            json!({"tools": TOOLS.iter().map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": input_schema(tool),
            })).collect::<Vec<_>>()}),
        ),
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
                return Some(rpc_error(id, -32602, &format!("Unknown tool: {name}")));
            };
            let arguments = message["params"]["arguments"].as_object().cloned().unwrap_or_default();
            let (is_error, text) = call_tool(state, headers, tool, arguments).await;
            rpc_result(id, json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
        }
        _ => rpc_error(id, -32601, "Method not found"),
    })
}

fn loopback_base(state: &AppState) -> String {
    let address = state.bind_address;
    let ip = if address.ip().is_unspecified() {
        "127.0.0.1".to_owned()
    } else if address.is_ipv6() {
        format!("[{}]", address.ip())
    } else {
        address.ip().to_string()
    };
    format!("http://{ip}:{}", address.port())
}

pub(super) fn build_request(
    tool: &Tool,
    mut arguments: Map<String, Value>,
) -> Result<(String, Vec<(String, String)>, Option<Value>), String> {
    let mut path = tool.path.to_owned();
    for param in tool.params {
        if !param.required {
            continue;
        }
        if !arguments.contains_key(param.name) {
            return Err(format!("Missing required argument: {}", param.name));
        }
    }
    for param in tool.params {
        let placeholder = format!("{{{}}}", param.name);
        if path.contains(&placeholder) {
            let value = arguments.remove(param.name).unwrap_or(Value::Null);
            let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
            if text.is_empty() || text.contains(['/', '?', '#', ' ']) {
                return Err(format!("Invalid value for {}", param.name));
            }
            path = path.replace(&placeholder, &text);
        }
    }
    let known: HashSet<&str> = tool.params.iter().map(|param| param.name).collect();
    if let Some(unknown) = arguments.keys().find(|key| !known.contains(key.as_str())) {
        return Err(format!("Unknown argument: {unknown}"));
    }
    if matches!(tool.method, "GET" | "DELETE") {
        let query = arguments
            .into_iter()
            .map(|(key, value)| {
                let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
                (key, text)
            })
            .collect();
        return Ok((path, query, None));
    }
    let mut body = Map::new();
    if let Some(Value::Object(data)) = arguments.remove("data") {
        body.extend(data);
    }
    body.extend(arguments);
    Ok((path, Vec::new(), Some(Value::Object(body))))
}

pub(super) async fn call_tool(
    state: &AppState,
    headers: &HeaderMap,
    tool: &Tool,
    arguments: Map<String, Value>,
) -> (bool, String) {
    let (path, query, body) = match build_request(tool, arguments) {
        Ok(parts) => parts,
        Err(message) => return (true, message),
    };
    let method = match tool.method {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        _ => reqwest::Method::DELETE,
    };
    let client = reqwest::Client::new();
    let mut request = client
        .request(method, format!("{}{path}", loopback_base(state)))
        .query(&query)
        .timeout(Duration::from_secs(180));
    if let Some(authorization) = headers.get(header::AUTHORIZATION) {
        request = request.header(reqwest::header::AUTHORIZATION, authorization.as_bytes());
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    match request.send().await {
        Ok(response) => {
            let failed = !response.status().is_success();
            let status = response.status();
            let mut text = response.text().await.unwrap_or_default();
            if text.len() > MAX_RESULT_BYTES {
                let mut cut = MAX_RESULT_BYTES;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                text.push_str("\n…[truncated]");
            }
            if text.is_empty() {
                text = format!("OK ({status})");
            } else if failed {
                text = format!("HTTP {status}: {text}");
            }
            (failed, text)
        }
        Err(error) => (true, format!("Request failed: {error}")),
    }
}

#[cfg(test)]
#[path = "tests/mcp_tests.rs"]
mod mcp_tests;
