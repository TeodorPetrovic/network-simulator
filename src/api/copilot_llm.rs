//! Provider adapters for the copilot agent. Messages are stored in one canonical shape
//! (Anthropic-style content blocks) and converted per provider at call time.

use super::*;

pub(super) struct ModelConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

pub(super) struct ModelReply {
    pub text: String,
    pub tool_calls: Vec<(String, String, Value)>, // (id, name, input)
}

pub(super) fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "https://api.anthropic.com",
        "ollama" => "http://127.0.0.1:11434/v1",
        _ => "https://api.openai.com/v1",
    }
}

pub(super) async fn complete(
    client: &reqwest::Client,
    config: &ModelConfig,
    system: &str,
    messages: &[Value],
    tools: &[Value],
) -> Result<ModelReply, String> {
    let base = if config.base_url.trim().is_empty() {
        default_base_url(&config.provider).to_owned()
    } else {
        config.base_url.trim_end_matches('/').to_owned()
    };
    if config.provider == "anthropic" {
        anthropic(client, config, &base, system, messages, tools).await
    } else {
        openai(client, config, &base, system, messages, tools).await
    }
}

async fn send(request: reqwest::RequestBuilder) -> Result<Value, String> {
    let response = request.send().await.map_err(|error| format!("Model request failed: {error}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let mut cut = text.len().min(600);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        return Err(format!("Model returned HTTP {status}: {}", &text[..cut]));
    }
    serde_json::from_str(&text).map_err(|error| format!("Model returned invalid JSON: {error}"))
}

async fn anthropic(
    client: &reqwest::Client,
    config: &ModelConfig,
    base: &str,
    system: &str,
    messages: &[Value],
    tools: &[Value],
) -> Result<ModelReply, String> {
    let tools: Vec<Value> = tools
        .iter()
        .map(|tool| json!({"name": tool["name"], "description": tool["description"], "input_schema": tool["inputSchema"]}))
        .collect();
    let body = json!({"model": config.model, "max_tokens": 4096, "system": system, "messages": messages, "tools": tools});
    let reply = send(
        client
            .post(format!("{base}/v1/messages"))
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body),
    )
    .await?;
    let mut out = ModelReply { text: String::new(), tool_calls: Vec::new() };
    for block in reply["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => out.text.push_str(block["text"].as_str().unwrap_or_default()),
            Some("tool_use") => out.tool_calls.push((
                block["id"].as_str().unwrap_or_default().to_owned(),
                block["name"].as_str().unwrap_or_default().to_owned(),
                block["input"].clone(),
            )),
            _ => {}
        }
    }
    Ok(out)
}

pub(super) fn to_openai_messages(system: &str, messages: &[Value]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];
    for message in messages {
        let blocks = message["content"].as_array().cloned().unwrap_or_default();
        if message["role"] == "assistant" {
            let text: String = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            let calls: Vec<Value> = blocks.iter().filter(|b| b["type"] == "tool_use").map(|b| json!({
                "id": b["id"], "type": "function",
                "function": {"name": b["name"], "arguments": b["input"].to_string()},
            })).collect();
            let mut entry = json!({"role": "assistant", "content": text});
            if !calls.is_empty() {
                entry["tool_calls"] = Value::Array(calls);
            }
            out.push(entry);
        } else {
            for block in &blocks {
                match block["type"].as_str() {
                    Some("tool_result") => out.push(json!({"role": "tool", "tool_call_id": block["tool_use_id"], "content": block["content"]})),
                    Some("text") => out.push(json!({"role": "user", "content": block["text"]})),
                    _ => {}
                }
            }
        }
    }
    out
}

async fn openai(
    client: &reqwest::Client,
    config: &ModelConfig,
    base: &str,
    system: &str,
    messages: &[Value],
    tools: &[Value],
) -> Result<ModelReply, String> {
    let tools: Vec<Value> = tools
        .iter()
        .map(|tool| json!({"type": "function", "function": {"name": tool["name"], "description": tool["description"], "parameters": tool["inputSchema"]}}))
        .collect();
    let body = json!({"model": config.model, "messages": to_openai_messages(system, messages), "tools": tools});
    let mut request = client.post(format!("{base}/chat/completions")).json(&body);
    if !config.api_key.is_empty() {
        request = request.bearer_auth(&config.api_key);
    }
    let reply = send(request).await?;
    let message = &reply["choices"][0]["message"];
    let mut out = ModelReply {
        text: message["content"].as_str().unwrap_or_default().to_owned(),
        tool_calls: Vec::new(),
    };
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        let input = call["function"]["arguments"]
            .as_str()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| json!({}));
        out.tool_calls.push((
            call["id"].as_str().unwrap_or_default().to_owned(),
            call["function"]["name"].as_str().unwrap_or_default().to_owned(),
            input,
        ));
    }
    Ok(out)
}
