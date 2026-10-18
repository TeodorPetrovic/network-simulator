CREATE TABLE controller_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    settings_json TEXT NOT NULL
);

INSERT INTO controller_settings (singleton, settings_json) VALUES (1, '{}');

CREATE TABLE controller_templates (
    template_id TEXT PRIMARY KEY NOT NULL,
    template_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE project_snapshots (
    snapshot_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    topology_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX project_snapshots_by_project ON project_snapshots(project_id, created_at, name);

CREATE TABLE project_files (
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    contents BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (project_id, file_path)
);

CREATE TABLE node_files (
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    contents BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (project_id, node_id, file_path)
);

CREATE TABLE controller_symbols (
    symbol_id TEXT PRIMARY KEY NOT NULL,
    contents BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
