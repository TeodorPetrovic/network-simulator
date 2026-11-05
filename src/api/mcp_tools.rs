//! Declarative MCP tool table. Every tool maps onto one REST call of this controller.
//!
//! Argument routing: `{name}` placeholders in `path` are filled from arguments; the remaining
//! arguments become the query string for GET/DELETE and the JSON body for POST/PUT/PATCH.
//! A `data` object argument (when declared) is merged into the body as-is.

pub(super) struct Param {
    pub name: &'static str,
    pub kind: &'static str,
    pub required: bool,
    pub description: &'static str,
}

pub(super) struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub params: &'static [Param],
}

const fn p(name: &'static str, kind: &'static str, description: &'static str) -> Param {
    Param { name, kind, required: true, description }
}
const fn o(name: &'static str, kind: &'static str, description: &'static str) -> Param {
    Param { name, kind, required: false, description }
}

const PROJECT: Param = p("project_id", "string", "Project UUID");
const NODE: Param = p("node_id", "string", "Node UUID");
const LINK: Param = p("link_id", "string", "Link UUID");
const DATA: Param = o("data", "object", "Fields to set, as in the REST API");

pub(super) const TOOLS: &[Tool] = &[
    Tool { name: "project_list", description: "List all projects the caller may see.", method: "GET", path: "/v3/projects", params: &[] },
    Tool { name: "project_get", description: "Get one project.", method: "GET", path: "/v3/projects/{project_id}", params: &[PROJECT] },
    Tool { name: "project_create", description: "Create a project.", method: "POST", path: "/v3/projects", params: &[p("name", "string", "Project name")] },
    Tool { name: "project_update", description: "Update project fields such as name, grid, scene size.", method: "PUT", path: "/v3/projects/{project_id}", params: &[PROJECT, DATA] },
    Tool { name: "project_delete", description: "Delete a project.", method: "DELETE", path: "/v3/projects/{project_id}", params: &[PROJECT] },
    Tool { name: "project_open", description: "Open a project so its nodes can run.", method: "POST", path: "/v3/projects/{project_id}/open", params: &[PROJECT] },
    Tool { name: "project_close", description: "Close a project and stop its runtimes.", method: "POST", path: "/v3/projects/{project_id}/close", params: &[PROJECT] },
    Tool { name: "project_stats", description: "Node, link and drawing counts for a project.", method: "GET", path: "/v3/projects/{project_id}/stats", params: &[PROJECT] },
    Tool { name: "project_duplicate", description: "Duplicate a project under a new name.", method: "POST", path: "/v3/projects/{project_id}/duplicate", params: &[PROJECT, p("name", "string", "Name of the copy")] },
    Tool { name: "node_list", description: "List nodes of a project.", method: "GET", path: "/v3/projects/{project_id}/nodes", params: &[PROJECT] },
    Tool { name: "node_get", description: "Get one node, including status, console and ports.", method: "GET", path: "/v3/projects/{project_id}/nodes/{node_id}", params: &[PROJECT, NODE] },
    Tool { name: "node_create_from_template", description: "Create a node from a template at a canvas position.", method: "POST", path: "/v3/projects/{project_id}/templates/{template_id}", params: &[PROJECT, p("template_id", "string", "Template UUID"), o("x", "integer", "Canvas x"), o("y", "integer", "Canvas y"), o("name", "string", "Optional node name")] },
    Tool { name: "node_create", description: "Create a node from raw fields (name, node_type, x, y, properties...).", method: "POST", path: "/v3/projects/{project_id}/nodes", params: &[PROJECT, p("name", "string", "Node name"), p("node_type", "string", "qemu, docker, vpcs, dynamips, ethernet_switch, ..."), DATA] },
    Tool { name: "node_update", description: "Update node fields (name, position, lock, properties).", method: "PUT", path: "/v3/projects/{project_id}/nodes/{node_id}", params: &[PROJECT, NODE, DATA] },
    Tool { name: "node_delete", description: "Delete a node and its links.", method: "DELETE", path: "/v3/projects/{project_id}/nodes/{node_id}", params: &[PROJECT, NODE] },
    Tool { name: "node_start", description: "Start a node.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/start", params: &[PROJECT, NODE] },
    Tool { name: "node_stop", description: "Stop a node.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/stop", params: &[PROJECT, NODE] },
    Tool { name: "node_suspend", description: "Suspend a node.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/suspend", params: &[PROJECT, NODE] },
    Tool { name: "node_reload", description: "Reload a node.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/reload", params: &[PROJECT, NODE] },
    Tool { name: "node_duplicate", description: "Duplicate a node at an offset position.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/duplicate", params: &[PROJECT, NODE, o("x", "integer", "Canvas x"), o("y", "integer", "Canvas y")] },
    Tool { name: "node_isolate", description: "Disconnect all links of a node without deleting them.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/isolate", params: &[PROJECT, NODE] },
    Tool { name: "node_unisolate", description: "Restore the links of an isolated node.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/unisolate", params: &[PROJECT, NODE] },
    Tool { name: "node_links", description: "List links attached to a node.", method: "GET", path: "/v3/projects/{project_id}/nodes/{node_id}/links", params: &[PROJECT, NODE] },
    Tool { name: "node_start_all", description: "Start every node in a project.", method: "POST", path: "/v3/projects/{project_id}/nodes/start", params: &[PROJECT] },
    Tool { name: "node_stop_all", description: "Stop every node in a project.", method: "POST", path: "/v3/projects/{project_id}/nodes/stop", params: &[PROJECT] },
    Tool { name: "node_suspend_all", description: "Suspend every node in a project.", method: "POST", path: "/v3/projects/{project_id}/nodes/suspend", params: &[PROJECT] },
    Tool { name: "node_run_command", description: "Run one command on a started QEMU device's serial console (login handled) and return its output. Use for show/export commands.", method: "POST", path: "/v3/projects/{project_id}/nodes/{node_id}/config/export", params: &[PROJECT, NODE, p("command", "string", "Command to run, e.g. /export or show running-config")] },
    Tool { name: "node_file_list", description: "List files in a node's directory.", method: "GET", path: "/v3/projects/{project_id}/nodes/{node_id}/files", params: &[PROJECT, NODE] },
    Tool { name: "link_list", description: "List links of a project.", method: "GET", path: "/v3/projects/{project_id}/links", params: &[PROJECT] },
    Tool { name: "link_get", description: "Get one link.", method: "GET", path: "/v3/projects/{project_id}/links/{link_id}", params: &[PROJECT, LINK] },
    Tool { name: "link_create", description: "Create a link. nodes = [{node_id, adapter_number, port_number}, {...}].", method: "POST", path: "/v3/projects/{project_id}/links", params: &[PROJECT, p("nodes", "array", "Two endpoints: [{node_id, adapter_number, port_number}]")] },
    Tool { name: "link_update", description: "Update link filters, suspend flag or label.", method: "PUT", path: "/v3/projects/{project_id}/links/{link_id}", params: &[PROJECT, LINK, DATA] },
    Tool { name: "link_delete", description: "Delete a link.", method: "DELETE", path: "/v3/projects/{project_id}/links/{link_id}", params: &[PROJECT, LINK] },
    Tool { name: "link_available_filters", description: "List packet filters usable on a link.", method: "GET", path: "/v3/projects/{project_id}/links/{link_id}/available_filters", params: &[PROJECT, LINK] },
    Tool { name: "link_capture_start", description: "Start a packet capture on a link.", method: "POST", path: "/v3/projects/{project_id}/links/{link_id}/capture/start", params: &[PROJECT, LINK, o("capture_file_name", "string", "Capture file name")] },
    Tool { name: "link_capture_stop", description: "Stop a packet capture on a link.", method: "POST", path: "/v3/projects/{project_id}/links/{link_id}/capture/stop", params: &[PROJECT, LINK] },
    Tool { name: "template_list", description: "List device templates.", method: "GET", path: "/v3/templates", params: &[] },
    Tool { name: "template_get", description: "Get one template.", method: "GET", path: "/v3/templates/{template_id}", params: &[p("template_id", "string", "Template UUID")] },
    Tool { name: "template_create", description: "Create a template.", method: "POST", path: "/v3/templates", params: &[p("name", "string", "Template name"), p("template_type", "string", "qemu, docker, vpcs, dynamips, ..."), DATA] },
    Tool { name: "template_update", description: "Update a template.", method: "PUT", path: "/v3/templates/{template_id}", params: &[p("template_id", "string", "Template UUID"), DATA] },
    Tool { name: "template_delete", description: "Delete a template.", method: "DELETE", path: "/v3/templates/{template_id}", params: &[p("template_id", "string", "Template UUID")] },
    Tool { name: "snapshot_list", description: "List snapshots of a project.", method: "GET", path: "/v3/projects/{project_id}/snapshots", params: &[PROJECT] },
    Tool { name: "snapshot_create", description: "Create a snapshot.", method: "POST", path: "/v3/projects/{project_id}/snapshots", params: &[PROJECT, p("name", "string", "Snapshot name")] },
    Tool { name: "snapshot_restore", description: "Restore a snapshot.", method: "POST", path: "/v3/projects/{project_id}/snapshots/{snapshot_id}/restore", params: &[PROJECT, p("snapshot_id", "string", "Snapshot UUID")] },
    Tool { name: "snapshot_delete", description: "Delete a snapshot.", method: "DELETE", path: "/v3/projects/{project_id}/snapshots/{snapshot_id}", params: &[PROJECT, p("snapshot_id", "string", "Snapshot UUID")] },
    Tool { name: "drawing_list", description: "List drawings (text, shapes, images) of a project.", method: "GET", path: "/v3/projects/{project_id}/drawings", params: &[PROJECT] },
    Tool { name: "drawing_create", description: "Create a drawing from raw fields (svg, x, y, z).", method: "POST", path: "/v3/projects/{project_id}/drawings", params: &[PROJECT, p("svg", "string", "SVG markup"), o("x", "integer", "Canvas x"), o("y", "integer", "Canvas y"), o("z", "integer", "Layer")] },
    Tool { name: "drawing_delete", description: "Delete a drawing.", method: "DELETE", path: "/v3/projects/{project_id}/drawings/{drawing_id}", params: &[PROJECT, p("drawing_id", "string", "Drawing UUID")] },
    Tool { name: "compute_list", description: "List compute servers.", method: "GET", path: "/v3/computes", params: &[] },
    Tool { name: "server_version", description: "Controller version information.", method: "GET", path: "/v3/version", params: &[] },
];

pub(super) fn input_schema(tool: &Tool) -> serde_json::Value {
    use serde_json::{Map, json};
    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in tool.params {
        properties.insert(
            param.name.to_owned(),
            json!({"type": param.kind, "description": param.description}),
        );
        if param.required {
            required.push(param.name);
        }
    }
    json!({"type": "object", "properties": properties, "required": required})
}
