CREATE TABLE auth_config (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    auth_enabled INTEGER NOT NULL DEFAULT 0 CHECK (auth_enabled IN (0, 1)),
    jwt_signing_key TEXT NOT NULL
);

CREATE TABLE auth_users (
    user_id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    email TEXT UNIQUE,
    full_name TEXT,
    password_hash TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    is_superadmin INTEGER NOT NULL DEFAULT 0 CHECK (is_superadmin IN (0, 1)),
    token_version INTEGER NOT NULL DEFAULT 0,
    last_login TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE auth_groups (
    user_group_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    is_builtin INTEGER NOT NULL DEFAULT 0 CHECK (is_builtin IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE auth_user_groups (
    user_id TEXT NOT NULL REFERENCES auth_users(user_id) ON DELETE CASCADE,
    user_group_id TEXT NOT NULL REFERENCES auth_groups(user_group_id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, user_group_id)
);

CREATE TABLE auth_roles (
    role_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    is_builtin INTEGER NOT NULL DEFAULT 0 CHECK (is_builtin IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE auth_privileges (
    privilege_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE auth_role_privileges (
    role_id TEXT NOT NULL REFERENCES auth_roles(role_id) ON DELETE CASCADE,
    privilege_id TEXT NOT NULL REFERENCES auth_privileges(privilege_id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, privilege_id)
);

CREATE TABLE auth_acl (
    ace_id TEXT PRIMARY KEY NOT NULL,
    ace_type TEXT NOT NULL CHECK (ace_type IN ('user', 'group')),
    path TEXT NOT NULL,
    propagate INTEGER NOT NULL DEFAULT 1 CHECK (propagate IN (0, 1)),
    allowed INTEGER NOT NULL DEFAULT 1 CHECK (allowed IN (0, 1)),
    user_id TEXT REFERENCES auth_users(user_id) ON DELETE CASCADE,
    group_id TEXT REFERENCES auth_groups(user_group_id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES auth_roles(role_id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((ace_type = 'user' AND user_id IS NOT NULL AND group_id IS NULL) OR
           (ace_type = 'group' AND group_id IS NOT NULL AND user_id IS NULL)),
    UNIQUE (ace_type, path, user_id, group_id, role_id)
);

CREATE TABLE resource_pools (
    resource_pool_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE resources (
    resource_id TEXT PRIMARY KEY NOT NULL,
    resource_type TEXT NOT NULL CHECK (resource_type = 'project'),
    name TEXT
);

CREATE TABLE resource_pool_map (
    resource_id TEXT NOT NULL REFERENCES resources(resource_id) ON DELETE CASCADE,
    resource_pool_id TEXT NOT NULL REFERENCES resource_pools(resource_pool_id) ON DELETE CASCADE,
    PRIMARY KEY (resource_id, resource_pool_id)
);

CREATE TABLE auth_api_keys (
    api_key_id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES auth_users(user_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    key_prefix TEXT NOT NULL,
    secret_hash TEXT NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at TEXT
);

CREATE INDEX auth_acl_path ON auth_acl(path);
CREATE INDEX auth_api_keys_user ON auth_api_keys(user_id);
