CREATE TABLE projects (
    project_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    project_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE topology_objects (
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('node', 'link', 'drawing')),
    object_id TEXT NOT NULL,
    name TEXT,
    object_json TEXT NOT NULL,
    PRIMARY KEY (project_id, kind, object_id)
);

CREATE UNIQUE INDEX topology_node_names
    ON topology_objects(project_id, name)
    WHERE kind = 'node' AND name IS NOT NULL;

CREATE TABLE link_endpoints (
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    link_id TEXT NOT NULL,
    endpoint_index INTEGER NOT NULL,
    node_id TEXT NOT NULL,
    adapter_number INTEGER NOT NULL,
    port_number INTEGER NOT NULL,
    PRIMARY KEY (project_id, link_id, endpoint_index)
);

CREATE UNIQUE INDEX link_endpoint_port
    ON link_endpoints(project_id, node_id, adapter_number, port_number);

CREATE INDEX link_endpoints_by_node
    ON link_endpoints(project_id, node_id, link_id);

CREATE TABLE project_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    action TEXT NOT NULL,
    event_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX project_events_by_project
    ON project_events(project_id, sequence);
