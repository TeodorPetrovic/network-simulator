use super::*;
use crate::api::mcp_tools::TOOLS;

fn tool(name: &str) -> &'static Tool {
    TOOLS.iter().find(|tool| tool.name == name).unwrap()
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn tool_names_are_unique() {
    let mut seen = HashSet::new();
    for tool in TOOLS {
        assert!(seen.insert(tool.name), "duplicate tool {}", tool.name);
    }
}

#[test]
fn path_arguments_are_substituted_and_rest_goes_to_body() {
    let (path, query, body) = build_request(
        tool("node_create_from_template"),
        args(json!({"project_id": "p1", "template_id": "t1", "x": 10, "y": 20})),
    )
    .unwrap();
    assert_eq!(path, "/v3/projects/p1/templates/t1");
    assert!(query.is_empty());
    assert_eq!(body.unwrap(), json!({"x": 10, "y": 20}));
}

#[test]
fn data_object_is_merged_into_body() {
    let (_, _, body) = build_request(
        tool("node_update"),
        args(json!({"project_id": "p", "node_id": "n", "data": {"name": "R1", "locked": true}})),
    )
    .unwrap();
    assert_eq!(body.unwrap(), json!({"name": "R1", "locked": true}));
}

#[test]
fn missing_unknown_and_unsafe_arguments_are_rejected() {
    assert!(build_request(tool("project_get"), args(json!({}))).is_err());
    assert!(build_request(tool("project_get"), args(json!({"project_id": "a", "bogus": 1}))).is_err());
    assert!(build_request(tool("project_get"), args(json!({"project_id": "../x"}))).is_err());
}

#[test]
fn get_arguments_become_query_string() {
    let (path, query, body) = build_request(tool("project_list"), Map::new()).unwrap();
    assert_eq!(path, "/v3/projects");
    assert!(query.is_empty() && body.is_none());
}
