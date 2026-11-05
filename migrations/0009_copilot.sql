CREATE TABLE llm_model_configs (
    config_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,
    base_url TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL,
    api_key TEXT NOT NULL DEFAULT '',
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX llm_model_configs_user ON llm_model_configs (user_id);

CREATE TABLE copilot_sessions (
    session_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'New chat',
    pinned INTEGER NOT NULL DEFAULT 0,
    messages TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX copilot_sessions_owner ON copilot_sessions (user_id, project_id);
