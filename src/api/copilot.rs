//! AI copilot: per-user model configurations, per-project chat sessions and the agent loop that
//! lets a model drive the simulator through the same tool table as the MCP server.

use super::copilot_llm::{ModelConfig, complete, default_base_url};
use super::mcp::call_tool;
use super::mcp_tools::{TOOLS, input_schema};
use super::*;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;

const MAX_AGENT_STEPS: usize = 14;
const MAX_MESSAGE_CHARS: usize = 20_000;
const PROVIDERS: [&str; 3] = ["openai", "anthropic", "ollama"];

const SYSTEM_PROMPT: &str = "You are the Network Simulator copilot. You help the user build, \
inspect and troubleshoot network labs by calling tools. Work on the project given below unless the \
user names another. Prefer reading state (node_list, link_list, node_get) before changing it. \
Before deleting anything or stopping a running lab, say what you will do and ask the user to \
confirm unless they already asked for exactly that. Use node_run_command only for show/inspection \
commands unless asked to configure. Be concise; summarise tool output instead of pasting it.";

type Result<T> = std::result::Result<T, ApiError>;

fn user_id(principal: &auth::Principal) -> &str {
    &principal.user_id
}

fn redact(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({
        "config_id": row.get::<String, _>("config_id"),
        "name": row.get::<String, _>("name"),
        "provider": row.get::<String, _>("provider"),
        "base_url": row.get::<String, _>("base_url"),
        "model": row.get::<String, _>("model"),
        "has_api_key": !row.get::<String, _>("api_key").is_empty(),
        "is_default": row.get::<i64, _>("is_default") != 0,
    })
}

fn text_field(body: &Value, key: &str, max: usize) -> Result<Option<String>> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.chars().count() <= max && !value.contains(['\n', '\r']) => {
            Ok(Some(value.trim().to_owned()))
        }
        _ => Err(ApiError::bad_request(format!("{key} must be a single-line string up to {max} characters"))),
    }
}

fn validate_base_url(url: &str) -> Result<()> {
    if url.is_empty() || url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(ApiError::bad_request("base_url must start with http:// or https://"))
    }
}

pub(super) async fn list_configs(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
) -> Result<Json<Value>> {
    let rows = sqlx::query("SELECT * FROM llm_model_configs WHERE user_id = ? ORDER BY created_at")
        .bind(user_id(&principal))
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(redact).collect())))
}

pub(super) async fn create_config(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>)> {
    let provider = text_field(&body, "provider", 32)?.unwrap_or_else(|| "openai".to_owned());
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(ApiError::bad_request("provider must be openai, anthropic or ollama"));
    }
    let model = text_field(&body, "model", 120)?.filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::bad_request("model is required"))?;
    let name = text_field(&body, "name", 80)?.filter(|v| !v.is_empty()).unwrap_or_else(|| model.clone());
    let base_url = text_field(&body, "base_url", 300)?.unwrap_or_default();
    validate_base_url(&base_url)?;
    let api_key = text_field(&body, "api_key", 500)?.unwrap_or_default();
    let config_id = Uuid::new_v4().to_string();
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM llm_model_configs WHERE user_id = ?")
        .bind(user_id(&principal))
        .fetch_one(state.store.pool())
        .await
        .map_err(db_error)?;
    let make_default = existing == 0 || body["is_default"].as_bool().unwrap_or(false);
    if make_default {
        sqlx::query("UPDATE llm_model_configs SET is_default = 0 WHERE user_id = ?")
            .bind(user_id(&principal)).execute(state.store.pool()).await.map_err(db_error)?;
    }
    sqlx::query("INSERT INTO llm_model_configs (config_id, user_id, name, provider, base_url, model, api_key, is_default) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&config_id).bind(user_id(&principal)).bind(&name).bind(&provider).bind(&base_url)
        .bind(&model).bind(&api_key).bind(make_default)
        .execute(state.store.pool()).await.map_err(db_error)?;
    let row = sqlx::query("SELECT * FROM llm_model_configs WHERE config_id = ?")
        .bind(&config_id).fetch_one(state.store.pool()).await.map_err(db_error)?;
    Ok((StatusCode::CREATED, Json(redact(&row))))
}

pub(super) async fn update_config(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path(config_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let id = config_id.to_string();
    let row = sqlx::query("SELECT * FROM llm_model_configs WHERE config_id = ? AND user_id = ?")
        .bind(&id).bind(user_id(&principal)).fetch_optional(state.store.pool()).await.map_err(db_error)?
        .ok_or_else(|| ApiError::not_found("Model configuration not found"))?;
    let provider = text_field(&body, "provider", 32)?.unwrap_or_else(|| row.get("provider"));
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(ApiError::bad_request("provider must be openai, anthropic or ollama"));
    }
    let model = text_field(&body, "model", 120)?.filter(|v| !v.is_empty()).unwrap_or_else(|| row.get("model"));
    let name = text_field(&body, "name", 80)?.filter(|v| !v.is_empty()).unwrap_or_else(|| row.get("name"));
    let base_url = text_field(&body, "base_url", 300)?.unwrap_or_else(|| row.get("base_url"));
    validate_base_url(&base_url)?;
    // An omitted or empty api_key keeps the stored one; the key is never returned to clients.
    let api_key = text_field(&body, "api_key", 500)?.filter(|v| !v.is_empty()).unwrap_or_else(|| row.get("api_key"));
    if body["is_default"].as_bool() == Some(true) {
        sqlx::query("UPDATE llm_model_configs SET is_default = 0 WHERE user_id = ?")
            .bind(user_id(&principal)).execute(state.store.pool()).await.map_err(db_error)?;
        sqlx::query("UPDATE llm_model_configs SET is_default = 1 WHERE config_id = ?")
            .bind(&id).execute(state.store.pool()).await.map_err(db_error)?;
    }
    sqlx::query("UPDATE llm_model_configs SET name=?, provider=?, base_url=?, model=?, api_key=? WHERE config_id=?")
        .bind(&name).bind(&provider).bind(&base_url).bind(&model).bind(&api_key).bind(&id)
        .execute(state.store.pool()).await.map_err(db_error)?;
    let row = sqlx::query("SELECT * FROM llm_model_configs WHERE config_id = ?")
        .bind(&id).fetch_one(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(redact(&row)))
}

pub(super) async fn delete_config(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path(config_id): Path<Uuid>,
) -> Result<StatusCode> {
    let done = sqlx::query("DELETE FROM llm_model_configs WHERE config_id = ? AND user_id = ?")
        .bind(config_id.to_string()).bind(user_id(&principal))
        .execute(state.store.pool()).await.map_err(db_error)?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Model configuration not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn list_sessions(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>> {
    let rows = sqlx::query("SELECT session_id, title, pinned, updated_at FROM copilot_sessions WHERE user_id = ? AND project_id = ? ORDER BY pinned DESC, updated_at DESC")
        .bind(user_id(&principal)).bind(project_id.to_string())
        .fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(|row| json!({
        "session_id": row.get::<String, _>("session_id"),
        "title": row.get::<String, _>("title"),
        "pinned": row.get::<i64, _>("pinned") != 0,
        "updated_at": row.get::<String, _>("updated_at"),
    })).collect())))
}

pub(super) async fn session_history(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path((project_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>> {
    let row = sqlx::query("SELECT messages FROM copilot_sessions WHERE session_id = ? AND project_id = ? AND user_id = ?")
        .bind(session_id.to_string()).bind(project_id.to_string()).bind(user_id(&principal))
        .fetch_optional(state.store.pool()).await.map_err(db_error)?
        .ok_or_else(|| ApiError::not_found("Chat session not found"))?;
    let messages: Value = serde_json::from_str(&row.get::<String, _>("messages")).unwrap_or_else(|_| json!([]));
    Ok(Json(json!({"session_id": session_id, "messages": messages})))
}

pub(super) async fn delete_session(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path((project_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode> {
    let done = sqlx::query("DELETE FROM copilot_sessions WHERE session_id = ? AND project_id = ? AND user_id = ?")
        .bind(session_id.to_string()).bind(project_id.to_string()).bind(user_id(&principal))
        .execute(state.store.pool()).await.map_err(db_error)?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Chat session not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn update_session(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path((project_id, session_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let title = text_field(&body, "title", 120)?.filter(|v| !v.is_empty());
    let pinned = body["pinned"].as_bool();
    let done = sqlx::query("UPDATE copilot_sessions SET title = COALESCE(?, title), pinned = COALESCE(?, pinned) WHERE session_id = ? AND project_id = ? AND user_id = ?")
        .bind(title).bind(pinned).bind(session_id.to_string()).bind(project_id.to_string()).bind(user_id(&principal))
        .execute(state.store.pool()).await.map_err(db_error)?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Chat session not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

fn db_error(error: sqlx::Error) -> ApiError {
    tracing::error!(%error, "copilot storage error");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Storage error")
}

fn event(kind: &str, data: Value) -> std::result::Result<Event, Infallible> {
    Ok(Event::default().event(kind).data(data.to_string()))
}

pub(super) async fn chat_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(principal): axum::Extension<auth::Principal>,
    Path(project_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Result<Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>> {
    let message = body["message"].as_str().map(str::trim).filter(|m| !m.is_empty())
        .ok_or_else(|| ApiError::bad_request("message is required"))?;
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(ApiError::bad_request("message is too long"));
    }
    let owner = user_id(&principal).to_owned();
    let row = match body["config_id"].as_str() {
        Some(id) => sqlx::query("SELECT * FROM llm_model_configs WHERE config_id = ? AND user_id = ?")
            .bind(id).bind(&owner).fetch_optional(state.store.pool()).await,
        None => sqlx::query("SELECT * FROM llm_model_configs WHERE user_id = ? ORDER BY is_default DESC, created_at LIMIT 1")
            .bind(&owner).fetch_optional(state.store.pool()).await,
    }
    .map_err(db_error)?
    .ok_or_else(|| ApiError::bad_request("No AI model is configured. Add one in the copilot settings first."))?;
    let provider: String = row.get("provider");
    let config = ModelConfig {
        base_url: row.get("base_url"),
        model: row.get("model"),
        api_key: row.get("api_key"),
        provider,
    };
    if config.api_key.is_empty() && config.provider != "ollama" && config.base_url.is_empty() {
        return Err(ApiError::bad_request("This model configuration has no API key."));
    }

    let project = project_id.to_string();
    let (session_id, mut messages, is_new) = match body["session_id"].as_str() {
        Some(id) => {
            let existing = sqlx::query("SELECT messages FROM copilot_sessions WHERE session_id = ? AND project_id = ? AND user_id = ?")
                .bind(id).bind(&project).bind(&owner).fetch_optional(state.store.pool()).await.map_err(db_error)?
                .ok_or_else(|| ApiError::not_found("Chat session not found"))?;
            let list: Vec<Value> = serde_json::from_str(&existing.get::<String, _>("messages")).unwrap_or_default();
            (id.to_owned(), list, false)
        }
        None => (Uuid::new_v4().to_string(), Vec::new(), true),
    };
    if is_new {
        let title: String = message.chars().take(60).collect();
        sqlx::query("INSERT INTO copilot_sessions (session_id, project_id, user_id, title) VALUES (?, ?, ?, ?)")
            .bind(&session_id).bind(&project).bind(&owner).bind(title.replace(['\n', '\r'], " "))
            .execute(state.store.pool()).await.map_err(db_error)?;
    }
    messages.push(json!({"role": "user", "content": [{"type": "text", "text": message}]}));
    let read_only = body["read_only"].as_bool().unwrap_or(false);
    let tools: Vec<&'static super::mcp_tools::Tool> = TOOLS.iter().filter(|t| !read_only || t.method == "GET").collect();
    let tool_defs: Vec<Value> = tools.iter().map(|t| json!({"name": t.name, "description": t.description, "inputSchema": input_schema(t)})).collect();
    let system = format!("{SYSTEM_PROMPT}\n\nCurrent project_id: {project}{}", if read_only { "\nRead-only mode: you cannot modify anything." } else { "" });

    let (tx, rx) = tokio::sync::mpsc::channel::<std::result::Result<Event, Infallible>>(64);
    let task_session = session_id.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().unwrap_or_default();
        let _ = tx.send(event("session", json!({"session_id": task_session}))).await;
        let mut failure: Option<String> = None;
        for _ in 0..MAX_AGENT_STEPS {
            let reply = match complete(&client, &config, &system, &messages, &tool_defs).await {
                Ok(reply) => reply,
                Err(error) => { failure = Some(error); break; }
            };
            let mut blocks = Vec::new();
            if !reply.text.is_empty() {
                blocks.push(json!({"type": "text", "text": reply.text}));
                let _ = tx.send(event("text", json!({"text": reply.text}))).await;
            }
            for (id, name, input) in &reply.tool_calls {
                blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
            }
            if blocks.is_empty() {
                blocks.push(json!({"type": "text", "text": ""}));
            }
            messages.push(json!({"role": "assistant", "content": blocks}));
            if reply.tool_calls.is_empty() {
                break;
            }
            let mut results = Vec::new();
            for (id, name, input) in reply.tool_calls {
                let _ = tx.send(event("tool_call", json!({"id": id, "name": name, "input": input}))).await;
                let (is_error, text) = match tools.iter().find(|t| t.name == name) {
                    Some(tool) => call_tool(&state, &headers, tool, input.as_object().cloned().unwrap_or_default()).await,
                    None => (true, format!("Unknown or unavailable tool: {name}")),
                };
                let preview: String = text.chars().take(2000).collect();
                let _ = tx.send(event("tool_result", json!({"id": id, "name": name, "is_error": is_error, "content": preview}))).await;
                results.push(json!({"type": "tool_result", "tool_use_id": id, "content": text, "is_error": is_error}));
            }
            messages.push(json!({"role": "user", "content": results}));
        }
        let _ = sqlx::query("UPDATE copilot_sessions SET messages = ?, updated_at = CURRENT_TIMESTAMP WHERE session_id = ?")
            .bind(Value::Array(messages).to_string()).bind(&task_session)
            .execute(state.store.pool()).await;
        match failure {
            Some(error) => { let _ = tx.send(event("error", json!({"message": error}))).await; }
            None => { let _ = tx.send(event("done", json!({}))).await; }
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
    let _ = default_base_url;
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
#[path = "tests/copilot_tests.rs"]
mod copilot_tests;
