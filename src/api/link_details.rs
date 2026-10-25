use super::*;

pub(super) async fn get_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let value = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "link",
        &link_id.to_string(),
    )
    .await?.0;
    Ok(etag_response(value))
}

pub(super) async fn update_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    headers: axum::http::HeaderMap,
    Json(patch): Json<Value>,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let current = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    check_if_match(&headers, &current)?;
    let patch = normalize_link_update(&current, patch)?;
    let mut candidate = current.clone();
    if let (Some(candidate), Some(patch)) = (candidate.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            candidate.insert(key.clone(), value.clone());
        }
    }
    if patch.get("filters").is_some() || patch.get("suspend").is_some() {
        let runtime_endpoints =
            link_runtime_endpoints(state.store.pool(), &project_id, &candidate).await?;
        for endpoint in candidate["nodes"].as_array().into_iter().flatten() {
            let Some(runtime_endpoint) = runtime_endpoints
                .iter()
                .find(|candidate| candidate["node_id"].as_str() == endpoint["node_id"].as_str())
            else {
                continue;
            };
            if runtime_endpoint_status(&state, &project_id, runtime_endpoint).await? == "stopped" {
                continue;
            }
            attach_runtime_link_endpoint(
                &state,
                &project_id,
                &candidate,
                endpoint,
                runtime_endpoint,
            )
            .await?;
        }
    }
    let updated = update_topology_object(state, project_id, link_id, "link", "link_id", patch).await?;
    Ok(etag_response(updated.0))
}

pub(super) fn normalize_link_update(current: &Value, patch: Value) -> Result<Value, ApiError> {
    let object = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("link update must be an object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "filters" | "suspend" | "link_style" | "nodes" | "show_filters_icon" | "label"
        ) {
            return Err(ApiError::bad_request(format!(
                "Link field '{key}' cannot be updated"
            )));
        }
    }
    let mut normalized = patch;
    if let Some(filters) = normalized.get("filters") {
        normalized["filters"] = validate_link_filters(filters)?;
    }
    if normalized
        .get("suspend")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(ApiError::bad_request("suspend must be a boolean"));
    }
    if normalized
        .get("link_style")
        .is_some_and(|value| !value.is_object())
    {
        return Err(ApiError::bad_request("link_style must be an object"));
    }
    if let Some(nodes) = normalized.get("nodes") {
        let requested = nodes
            .as_array()
            .filter(|nodes| nodes.len() == 2)
            .ok_or_else(|| ApiError::bad_request("GNS3 links have exactly two endpoints"))?;
        let mut merged = current["nodes"]
            .as_array()
            .cloned()
            .ok_or_else(|| ApiError::conflict("Stored link has invalid endpoints"))?;
        for requested_endpoint in requested {
            let node_id = requested_endpoint["node_id"]
                .as_str()
                .ok_or_else(|| ApiError::bad_request("Link endpoint requires node_id"))?;
            let existing = merged
                .iter_mut()
                .find(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
                .ok_or_else(|| {
                    ApiError::conflict("Delete and recreate the link to change runtime endpoints")
                })?;
            for identity in ["adapter_number", "port_number"] {
                if requested_endpoint
                    .get(identity)
                    .is_some_and(|value| value != &existing[identity])
                {
                    return Err(ApiError::conflict(
                        "Delete and recreate the link to change runtime endpoints",
                    ));
                }
            }
            if let Some(label) = requested_endpoint.get("label") {
                existing["label"] = label.clone();
            }
        }
        normalized["nodes"] = json!(merged);
    }
    Ok(normalized)
}

pub(super) fn validate_link_filters(filters: &Value) -> Result<Value, ApiError> {
    let filters = filters
        .as_object()
        .ok_or_else(|| ApiError::bad_request("filters must be an object"))?;
    let mut active = serde_json::Map::new();
    for (kind, values) in filters {
        let values = values
            .as_array()
            .ok_or_else(|| ApiError::bad_request(format!("filter '{kind}' must be an array")))?;
        match kind.as_str() {
            "frequency_drop" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("frequency_drop expects one value"));
                }
                let frequency = filter_integer(kind, &values[0], -1, 32767)?;
                if frequency != 0 {
                    active.insert(kind.clone(), json!([frequency]));
                }
            }
            "packet_loss" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("packet_loss expects one value"));
                }
                let chance = filter_integer(kind, &values[0], 0, 100)?;
                if chance != 0 {
                    active.insert(kind.clone(), json!([chance]));
                }
            }
            "delay" => {
                if !(1..=3).contains(&values.len()) {
                    return Err(ApiError::bad_request(
                        "delay expects latency, optional jitter, and optional distribution",
                    ));
                }
                let latency = filter_integer(kind, &values[0], 0, 32767)?;
                let jitter = if values.len() > 1 {
                    filter_integer(kind, &values[1], 0, 32767)?
                } else {
                    0
                };
                if latency == 0 && jitter == 0 {
                    continue;
                }
                if latency == 0 {
                    return Err(ApiError::bad_request(
                        "delay latency must be at least 1 ms when jitter is enabled",
                    ));
                }
                let mut normalized = vec![json!(latency), json!(jitter)];
                if let Some(distribution) = values.get(2) {
                    let distribution = distribution
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase();
                    if !matches!(
                        distribution.as_str(),
                        "" | "uniform" | "normal" | "pareto" | "paretonormal"
                    ) {
                        return Err(ApiError::bad_request(
                            "delay distribution must be uniform, normal, pareto, or paretonormal",
                        ));
                    }
                    if distribution.is_empty() {
                        normalized.push(json!(""));
                    } else {
                        normalized.push(json!(distribution));
                    }
                }
                active.insert(kind.clone(), Value::Array(normalized));
            }
            "corrupt" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("corrupt expects one value"));
                }
                let chance = filter_integer(kind, &values[0], 0, 100)?;
                if chance != 0 {
                    active.insert(kind.clone(), json!([chance]));
                }
            }
            "bpf" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("bpf expects one expression"));
                }
                let expression = values[0]
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request("bpf expression must be text"))?;
                if expression.len() > 4096 || expression.chars().any(char::is_control) {
                    return Err(ApiError::bad_request(
                        "bpf expression is too long or contains a control character",
                    ));
                }
                if !expression.trim().is_empty() {
                    active.insert(kind.clone(), json!([expression]));
                }
            }
            _ => {
                return Err(ApiError::bad_request(format!(
                    "Unsupported link filter '{kind}'"
                )));
            }
        }
    }
    Ok(Value::Object(active))
}

pub(super) fn filter_integer(
    kind: &str,
    value: &Value,
    minimum: i64,
    maximum: i64,
) -> Result<i64, ApiError> {
    let integer = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        .ok_or_else(|| ApiError::bad_request(format!("{kind} filter values must be integers")))?;
    if integer < minimum || integer > maximum {
        return Err(ApiError::bad_request(format!(
            "{kind} filter values must be between {minimum} and {maximum}"
        )));
    }
    Ok(integer)
}

pub(super) async fn available_link_filters(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let _ = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "link",
        &link_id.to_string(),
    )
    .await?;
    Ok(Json(vec![
        json!({ "type": "frequency_drop", "name": "Frequency drop", "description": "Drop every Nth packet; -1 drops all packets.", "parameters": [{"name":"Frequency","minimum":-1,"maximum":32767,"type":"int","unit":"th packet"}] }),
        json!({ "type": "packet_loss", "name": "Packet loss", "description": "Chance for a packet to be lost.", "parameters": [{"name":"Chance","minimum":0,"maximum":100,"type":"int","unit":"%"}] }),
        json!({ "type": "delay", "name": "Delay", "description": "Delay packets with optional jitter and distribution.", "parameters": [{"name":"Latency","minimum":1,"maximum":32767,"type":"int","unit":"ms"},{"name":"Jitter (-/+)","minimum":0,"maximum":32767,"type":"int","unit":"ms"},{"name":"Distribution","type":"str","unit":"uniform|normal|pareto|paretonormal"}] }),
        json!({ "type": "corrupt", "name": "Corrupt", "description": "Chance for a packet to be corrupted.", "parameters": [{"name":"Chance","minimum":0,"maximum":100,"type":"int","unit":"%"}] }),
        json!({ "type": "bpf", "name": "Berkeley Packet Filter (BPF)", "description": "Drop packets matching a BPF expression.", "parameters": [{"name":"Filters","type":"text"}] }),
    ]))
}
