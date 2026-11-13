//! Writing and reading `project.gns3` in the dialect of a specific GNS3 edition.
//!
//! GNS3 validates the topology file strictly: unknown keys are rejected, a node's top-level keys
//! are applied to `Node` attributes (so a stray `status` aborts the load), and node properties
//! are forwarded to compute schemas that also reject unknown keys. The app stores a superset, so
//! the exporter reduces it to what the target edition defines, and the importer maps the
//! edition's spellings back.
//!
//! Targets:
//! * `2.2` (default): revision 9. GNS3 2.2.x opens it, and GNS3 3.x upgrades revision 9 on load.
//! * `3.1`: revision 10, the native 3.x format.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::ApiError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    V22,
    V3,
}

impl Target {
    pub(super) fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(Self::V22),
            Some(v) if v == "2" || v.starts_with("2.") => Ok(Self::V22),
            Some(v) if v == "3" || v.starts_with("3.") => Ok(Self::V3),
            Some(other) => Err(ApiError::bad_request(format!(
                "Unsupported gns3_version '{other}'; use 2.2 or 3.1"
            ))),
        }
    }

    fn revision(self) -> i64 {
        match self {
            Self::V22 => 9,
            Self::V3 => 10,
        }
    }
}

const PROJECT_KEYS: &[&str] = &[
    "auto_close", "auto_open", "auto_start", "drawing_grid_size", "grid_size", "name", "project_id",
    "revision", "scene_height", "scene_width", "show_grid", "show_interface_labels", "show_layers",
    "snap_to_grid", "supplier", "topology", "type", "variables", "version", "zoom",
];
/// What `Node.__json__(topology_dump=True)` writes in GNS3 itself.
const NODE_KEYS: &[&str] = &[
    "compute_id", "console", "console_auto_start", "console_type", "custom_adapters", "first_port_name",
    "height", "label", "locked", "name", "node_id", "node_type", "port_name_format", "port_segment_size",
    "properties", "symbol", "template_id", "width", "x", "y", "z",
];
const LINK_KEYS: &[&str] = &[
    "capture_compute_id", "capture_file_name", "capture_file_path", "capturing", "filters", "link_id",
    "link_style", "link_type", "nodes", "suspend",
];
const LINK_NODE_KEYS: &[&str] = &["adapter_number", "label", "node_id", "port_number"];
const DRAWING_KEYS: &[&str] = &["drawing_id", "locked", "rotation", "svg", "x", "y", "z"];

// GNS3 2.2 compute "create" schemas, minus the keys the controller supplies itself.
const DOCKER: &[&str] = &[
    "adapters", "aux", "console_http_path", "console_http_port", "console_resolution", "console_type",
    "container_id", "custom_adapters", "environment", "extra_hosts", "extra_volumes", "image",
    "mac_address", "start_command", "usage",
];
const DYNAMIPS: &[&str] = &[
    "auto_delete_disks", "aux", "chassis", "clock_divisor", "disk0", "disk1", "dynamips_id", "exec_area",
    "idlemax", "idlepc", "idlesleep", "image", "image_md5sum", "iomem", "mac_addr", "midplane", "mmap",
    "npe", "nvram", "platform", "power_supplies", "private_config_content", "ram", "sensors", "slot0",
    "slot1", "slot2", "slot3", "slot4", "slot5", "slot6", "sparsemem", "startup_config_content",
    "system_id", "usage", "wic0", "wic1", "wic2",
];
const QEMU: &[&str] = &[
    "adapter_type", "adapters", "bios_image", "bios_image_md5sum", "boot_priority", "cdrom_image",
    "cdrom_image_md5sum", "cpu_throttling", "cpus", "create_config_disk", "custom_adapters",
    "hda_disk_image", "hda_disk_image_md5sum", "hda_disk_interface", "hdb_disk_image",
    "hdb_disk_image_md5sum", "hdb_disk_interface", "hdc_disk_image", "hdc_disk_image_md5sum",
    "hdc_disk_interface", "hdd_disk_image", "hdd_disk_image_md5sum", "hdd_disk_interface", "initrd",
    "initrd_md5sum", "kernel_command_line", "kernel_image", "kernel_image_md5sum", "legacy_networking",
    "linked_clone", "mac_address", "on_close", "options", "platform", "process_priority", "qemu_path",
    "ram", "replicate_network_connection_state", "tpm", "uefi", "usage",
];
const VPCS: &[&str] = &["startup_script"];
const PORTS_ONLY: &[&str] = &["ports_mapping"];
const CLOUD: &[&str] = &[
    "interfaces", "ports_mapping", "remote_console_host", "remote_console_http_path",
    "remote_console_port", "remote_console_type",
];

fn default_symbol(node_type: &str) -> &'static str {
    match node_type {
        "docker" => ":/symbols/docker_guest.svg",
        "vpcs" => ":/symbols/vpcs_guest.svg",
        "qemu" => ":/symbols/qemu_guest.svg",
        "dynamips" => ":/symbols/router.svg",
        "ethernet_switch" => ":/symbols/ethernet_switch.svg",
        "ethernet_hub" => ":/symbols/hub.svg",
        "nat" | "cloud" => ":/symbols/cloud.svg",
        _ => ":/symbols/computer.svg",
    }
}

/// GNS3 derives its built-in template ids as uuid3(NAMESPACE_DNS, <type>), so these match
/// every GNS3 install; a node keeps its template (and icon) when the project is opened there.
const GNS3_BUILTIN_IDS: &[(&str, &str)] = &[
    ("cloud", "39e257dc-8412-3174-b6b3-0ee3ed6a43e9"),
    ("nat", "df8f4ea9-33b7-3e96-86a2-c39bc9bb649c"),
    ("vpcs", "19021f99-e36f-394d-b4a1-8aaa902ab9cc"),
    ("ethernet_switch", "1966b864-93e7-32d5-965f-001384eec461"),
    ("ethernet_hub", "b4503ea9-d6b6-3695-9fe4-1db3b39290b0"),
];

fn gns3_builtin_id(node_type: &str) -> Option<&'static str> {
    GNS3_BUILTIN_IDS.iter().find(|(t, _)| *t == node_type).map(|(_, id)| *id)
}

/// `:/symbols/<name>.svg`, the form the GNS3 GUI resolves to a built-in icon.
fn normalise_symbol(symbol: &str) -> String {
    if symbol.starts_with(":/") {
        return symbol.to_owned();
    }
    let name = symbol.trim_start_matches('/').trim_start_matches("symbols/");
    if name.ends_with(".svg") || name.ends_with(".png") {
        format!(":/symbols/{name}")
    } else {
        format!(":/symbols/{name}.svg")
    }
}

const LABEL_STYLE: &str = "font-family: TypeWriter;font-size: 10.0;font-weight: bold;fill: #000000;fill-opacity: 1.0;";

fn non_empty_string(value: &Value) -> bool {
    value.as_str().is_some_and(|s| !s.is_empty())
}

/// Properties as GNS3 2.2 names them for each node type; `None` leaves the type untouched.
fn properties_v22(node_type: &str, props: &mut Map<String, Value>) -> Option<&'static [&'static str]> {
    match node_type {
        "docker" => Some(DOCKER),
        "dynamips" => {
            // 2.2 writes the config to the node's configs/ directory when it creates the router.
            for (from, to) in [
                ("startup_config", "startup_config_content"),
                ("private_config", "private_config_content"),
            ] {
                if let Some(value) = props.remove(from) {
                    if non_empty_string(&value) {
                        props.entry(to.to_owned()).or_insert(value);
                    }
                }
            }
            Some(DYNAMIPS)
        }
        "qemu" => {
            if let Some(memory) = props.get("memory").cloned() {
                props.entry("ram".to_owned()).or_insert(memory);
            }
            Some(QEMU)
        }
        "vpcs" => Some(VPCS),
        "ethernet_switch" | "ethernet_hub" | "nat" => Some(PORTS_ONLY),
        "cloud" => Some(CLOUD),
        _ => None,
    }
}

fn retain_keys(object: &mut Map<String, Value>, allowed: &[&str]) {
    object.retain(|key, _| allowed.contains(&key.as_str()));
}

fn convert_node(node: &mut Value, target: Target, symbols: &HashMap<String, String>) {
    let node_type = node.get("node_type").and_then(Value::as_str).unwrap_or_default().to_owned();
    let Some(object) = node.as_object_mut() else { return };
    let top = object.clone();
    let mut properties = object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // Docker keeps its HTTP/VNC console settings inside properties in GNS3 files.
    if node_type == "docker" {
        for key in ["console_http_path", "console_http_port", "console_resolution"] {
            if let Some(value) = top.get(key) {
                properties.entry(key.to_owned()).or_insert_with(|| value.clone());
            }
        }
    }
    if target == Target::V22 {
        if let Some(allowed) = properties_v22(&node_type, &mut properties) {
            retain_keys(&mut properties, allowed);
            // The controller drops nulls before forwarding, and the compute schemas often
            // reject them, so leave them out of the file.
            properties.retain(|_, value| !value.is_null());
        }
    }
    object.insert("properties".into(), Value::Object(properties));
    retain_keys(object, NODE_KEYS);
    object.retain(|_, value| !value.is_null());
    if let Some(id) = gns3_builtin_id(&node_type) {
        object.insert("template_id".into(), json!(id));
    }
    if !object.contains_key("symbol") {
        let from_template = object
            .get("template_id")
            .and_then(Value::as_str)
            .and_then(|id| symbols.get(id));
        let symbol = from_template.map_or_else(|| default_symbol(&node_type).to_owned(), |s| normalise_symbol(s));
        object.insert("symbol".into(), json!(symbol));
    }
    if !object.contains_key("label") {
        let text = object.get("name").cloned().unwrap_or_else(|| json!(""));
        object.insert("label".into(), json!({"text": text, "style": LABEL_STYLE, "x": null, "y": -25, "rotation": 0}));
    }
}

/// GNS3 numbers a Docker node's Ethernet ports as adapter N / port 0; this app keeps them as
/// adapter 0 / port N. Swapping the two converts the link endpoints of `docker` nodes in either
/// direction (the swap is its own inverse).
pub(super) fn docker_link_endpoints(nodes: &[Value], links: &mut [Value]) {
    let docker: std::collections::HashSet<&str> = nodes
        .iter()
        .filter(|node| node.get("node_type").and_then(Value::as_str) == Some("docker"))
        .filter_map(|node| node.get("node_id").and_then(Value::as_str))
        .collect();
    for link in links {
        for endpoint in link.get_mut("nodes").and_then(Value::as_array_mut).into_iter().flatten() {
            if !endpoint.get("node_id").and_then(Value::as_str).is_some_and(|id| docker.contains(id)) {
                continue;
            }
            let adapter = endpoint["adapter_number"].as_u64().unwrap_or(0);
            let port = endpoint["port_number"].as_u64().unwrap_or(0);
            endpoint["adapter_number"] = json!(port);
            endpoint["port_number"] = json!(adapter);
        }
    }
}

/// Turn the app's own export into a topology file GNS3 `target` loads.
pub(super) fn convert(project: &mut Value, project_id: &str, target: Target, symbols: &HashMap<String, String>) {
    project["project_id"] = json!(project_id);
    project["type"] = json!("topology");
    project["revision"] = json!(target.revision());
    if let Some(topology) = project.get_mut("topology") {
        let node_list = topology.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        if let Some(links) = topology.get_mut("links").and_then(Value::as_array_mut) {
            docker_link_endpoints(&node_list, links);
        }
        for node in topology.get_mut("nodes").and_then(Value::as_array_mut).into_iter().flatten() {
            convert_node(node, target, symbols);
        }
        for link in topology.get_mut("links").and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(object) = link.as_object_mut() {
                retain_keys(object, LINK_KEYS);
            }
            for endpoint in link.get_mut("nodes").and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(object) = endpoint.as_object_mut() {
                    retain_keys(object, LINK_NODE_KEYS);
                }
            }
        }
        for drawing in topology.get_mut("drawings").and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(object) = drawing.as_object_mut() {
                retain_keys(object, DRAWING_KEYS);
            }
        }
    }
    if let Some(object) = project.as_object_mut() {
        retain_keys(object, PROJECT_KEYS);
        object.retain(|_, value| !value.is_null());
    }
}

/// Map a node from a GNS3 file (either edition) back to the app's property names.
pub(super) fn import_node(node: &Value, builtin_ids: &HashMap<String, String>) -> Value {
    let mut node = node.clone();
    let node_type = node.get("node_type").and_then(Value::as_str).unwrap_or_default().to_owned();
    let Some(object) = node.as_object_mut() else { return node };
    // The importer may hand over the node flattened (properties merged into the node) or in the
    // file's nested form; apply the edition's spellings to whichever map holds the key.
    fn rename(map: &mut Map<String, Value>, from: &str, to: &str) {
        if let Some(value) = map.remove(from) {
            map.entry(to.to_owned()).or_insert(value);
        }
    }
    fn copy(map: &mut Map<String, Value>, from: &str, to: &str) {
        if let Some(value) = map.get(from).cloned() {
            map.entry(to.to_owned()).or_insert(value);
        }
    }
    if let Some(id) = object.get("template_id").and_then(Value::as_str) {
        // GNS3's built-in template ids stand for this app's own built-in templates.
        if let Some((kind, _)) = GNS3_BUILTIN_IDS.iter().find(|(_, gid)| *gid == id) {
            if let Some(ours) = builtin_ids.get(*kind) {
                object.insert("template_id".into(), json!(ours));
            }
        }
    }
    let mut nested = object.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    for map in [&mut nested, &mut *object] {
        match node_type.as_str() {
            "dynamips" => {
                rename(map, "startup_config_content", "startup_config");
                rename(map, "private_config_content", "private_config");
            }
            "qemu" => copy(map, "ram", "memory"),
            _ => {}
        }
    }
    if node_type == "docker" {
        for key in ["console_http_path", "console_http_port", "console_resolution"] {
            if let Some(value) = nested.get(key).cloned() {
                object.entry(key.to_owned()).or_insert(value);
            }
        }
    }
    if object.get("properties").is_some_and(|p| p.as_object().is_some_and(|m| !m.is_empty())) {
        object.insert("properties".into(), Value::Object(nested));
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "name": "lab",
            "topology": {
                "nodes": [
                    {"node_id": "n1", "node_type": "docker", "name": "A", "status": "stopped", "usage": "x",
                     "console_http_port": 80, "properties": {"image": "alpine", "cpus": 0.0, "memory": 0, "adapters": 1}},
                    {"node_id": "n2", "node_type": "dynamips", "name": "R", "properties":
                     {"startup_config": "hostname R", "private_config": "", "memory": 128, "ram": 128, "adapters": 1, "slot1": null}},
                    {"node_id": "n3", "node_type": "qemu", "name": "Q", "properties": {"memory": 384, "maxcpus": 1}},
                    {"node_id": "n4", "node_type": "vpcs", "name": "P", "properties": {"startup_script": "ip 1.1.1.1", "cpus": 0}}
                ],
                "links": [{"link_id": "l", "wireshark": false, "filter_endpoint_id": "x", "nodes": [{"node_id": "n1", "adapter_number": 0, "port_number": 0, "runtime_endpoint_id": "r"}]}],
                "drawings": []
            }
        })
    }

    #[test]
    fn v22_export_matches_gns3_schema() {
        let mut project = sample();
        convert(&mut project, "11111111-1111-1111-1111-111111111111", Target::V22, &HashMap::new());
        assert_eq!(project["revision"], 9);
        let nodes = project["topology"]["nodes"].as_array().unwrap();
        assert!(nodes[0].get("status").is_none() && nodes[0].get("usage").is_none());
        assert_eq!(nodes[0]["properties"]["console_http_port"], 80);
        assert!(nodes[0]["properties"].get("cpus").is_none());
        assert_eq!(nodes[1]["properties"]["startup_config_content"], "hostname R");
        assert!(nodes[1]["properties"].get("private_config_content").is_none());
        assert!(nodes[1]["properties"].get("slot1").is_none());
        assert!(nodes[1]["properties"].get("adapters").is_none());
        assert_eq!(nodes[2]["properties"]["ram"], 384);
        assert!(nodes[2]["properties"].get("maxcpus").is_none());
        assert_eq!(nodes[3]["properties"], json!({"startup_script": "ip 1.1.1.1"}));
        let link = &project["topology"]["links"][0];
        assert!(link.get("wireshark").is_none() && link.get("filter_endpoint_id").is_none());
        assert!(link["nodes"][0].get("runtime_endpoint_id").is_none());
    }

    #[test]
    fn v3_export_keeps_native_properties() {
        let mut project = sample();
        convert(&mut project, "11111111-1111-1111-1111-111111111111", Target::V3, &HashMap::new());
        assert_eq!(project["revision"], 10);
        let nodes = project["topology"]["nodes"].as_array().unwrap();
        assert!(nodes[0].get("status").is_none());
        assert_eq!(nodes[1]["properties"]["startup_config"], "hostname R");
    }

    #[test]
    fn import_restores_app_property_names() {
        let mut project = sample();
        convert(&mut project, "11111111-1111-1111-1111-111111111111", Target::V22, &HashMap::new());
        let router = import_node(&project["topology"]["nodes"][1], &HashMap::new());
        assert_eq!(router["properties"]["startup_config"], "hostname R");
        let qemu = import_node(&project["topology"]["nodes"][2], &HashMap::new());
        assert_eq!(qemu["properties"]["memory"], 384);
        let docker = import_node(&project["topology"]["nodes"][0], &HashMap::new());
        assert_eq!(docker["console_http_port"], 80);
    }

    #[test]
    fn builtin_ids_and_symbols_follow_gns3() {
        let mut project = json!({"topology": {"nodes": [
            {"node_id": "a", "node_type": "vpcs", "name": "PC1", "template_id": "ours-vpcs", "properties": {}},
            {"node_id": "b", "node_type": "docker", "name": "OVS", "template_id": "t-ovs", "properties": {"image": "x"}}
        ], "links": [], "drawings": []}});
        let symbols = HashMap::from([("t-ovs".to_owned(), "multilayer_switch.svg".to_owned())]);
        convert(&mut project, "11111111-1111-1111-1111-111111111111", Target::V22, &symbols);
        let nodes = project["topology"]["nodes"].as_array().unwrap();
        assert_eq!(nodes[0]["template_id"], "19021f99-e36f-394d-b4a1-8aaa902ab9cc");
        assert_eq!(nodes[0]["symbol"], ":/symbols/vpcs_guest.svg");
        assert_eq!(nodes[1]["symbol"], ":/symbols/multilayer_switch.svg");
        assert_eq!(nodes[1]["label"]["text"], "OVS");
        let ids = HashMap::from([("vpcs".to_owned(), "ours-vpcs".to_owned())]);
        assert_eq!(import_node(&nodes[0], &ids)["template_id"], "ours-vpcs");
    }

    #[test]
    fn import_handles_flattened_nodes() {
        let flat = json!({"node_type": "dynamips", "name": "R", "startup_config_content": "hostname R", "properties": {}});
        let node = import_node(&flat, &HashMap::new());
        assert_eq!(node["startup_config"], "hostname R");
        assert!(node.get("startup_config_content").is_none());
        let qemu = import_node(&json!({"node_type": "qemu", "ram": 384, "properties": {}}), &HashMap::new());
        assert_eq!(qemu["memory"], 384);
    }

    #[test]
    fn target_parsing() {
        assert_eq!(Target::parse(None).unwrap(), Target::V22);
        assert_eq!(Target::parse(Some("2.2.61")).unwrap(), Target::V22);
        assert_eq!(Target::parse(Some("3.1")).unwrap(), Target::V3);
        assert!(Target::parse(Some("1.5")).is_err());
    }
}

#[cfg(test)]
mod docker_port_tests {
    use super::*;

    #[test]
    fn docker_endpoints_swap_between_gns3_and_app_numbering() {
        let nodes = vec![json!({"node_id": "d", "node_type": "docker"}), json!({"node_id": "v", "node_type": "vpcs"})];
        let mut links = vec![json!({"nodes": [
            {"node_id": "d", "adapter_number": 3, "port_number": 0},
            {"node_id": "v", "adapter_number": 0, "port_number": 0}]})];
        docker_link_endpoints(&nodes, &mut links);
        assert_eq!(links[0]["nodes"][0]["adapter_number"], 0);
        assert_eq!(links[0]["nodes"][0]["port_number"], 3);
        assert_eq!(links[0]["nodes"][1]["port_number"], 0);
        docker_link_endpoints(&nodes, &mut links);
        assert_eq!(links[0]["nodes"][0]["adapter_number"], 3);
    }
}
