ALTER TABLE auth_users
    ADD COLUMN must_change_password INTEGER NOT NULL DEFAULT 0
    CHECK (must_change_password IN (0, 1));

CREATE TABLE auth_refresh_families (
    family_id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES auth_users(user_id) ON DELETE CASCADE,
    token_version INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE auth_refresh_tokens (
    jti TEXT PRIMARY KEY NOT NULL,
    family_id TEXT NOT NULL REFERENCES auth_refresh_families(family_id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX auth_refresh_tokens_family ON auth_refresh_tokens(family_id);

CREATE TABLE auth_ws_tickets (
    ticket_hash TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES auth_users(user_id) ON DELETE CASCADE,
    token_version INTEGER NOT NULL,
    path TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX auth_ws_tickets_expiry ON auth_ws_tickets(expires_at);
