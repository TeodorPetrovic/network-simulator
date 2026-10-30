use super::*;

pub(super) async fn list_symbols(
    State(state): State<AppState>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows=sqlx::query("SELECT symbol_id,length(contents) AS size,updated_at FROM controller_symbols ORDER BY symbol_id").fetch_all(state.store.pool()).await?;
    Ok(Json(rows.into_iter().map(|r|json!({"symbol_id":r.get::<String,_>("symbol_id"),"filename":r.get::<String,_>("symbol_id"),"size":r.get::<i64,_>("size"),"builtin":false})).collect()))
}
pub(super) async fn default_symbols() -> Json<Value> {
    Json(
        json!({"default_symbol":":/symbols/router.svg","symbols":{"router":"router.svg","switch":"switch.svg","cloud":"cloud.svg","pc":"pc.svg"}}),
    )
}
pub(super) fn safe_symbol_id(id: &str) -> Result<String, ApiError> {
    let name = Path::new(id)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ApiError::bad_request("symbol ID must be a filename"))?;
    if name != id
        || name.starts_with('.')
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ApiError::bad_request("symbol ID must be a safe filename"));
    }
    Ok(name.to_owned())
}
pub(super) async fn get_symbol(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let row = sqlx::query("SELECT contents FROM controller_symbols WHERE symbol_id=?")
        .bind(&id)
        .fetch_optional(state.store.pool())
        .await?
        .ok_or_else(|| ApiError::not_found("Symbol does not exist"))?;
    let mut response = (
        [(header::CONTENT_TYPE, content_type(&symbol_id))],
        Body::from(row.get::<Vec<u8>, _>("contents")),
    )
        .into_response();
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{id}\""))
            .expect("safe symbol IDs produce valid disposition headers"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'",
        ),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
pub(super) async fn upload_symbol(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let bytes = to_bytes(request.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    sqlx::query("INSERT INTO controller_symbols(symbol_id,contents) VALUES(?,?) ON CONFLICT(symbol_id) DO UPDATE SET contents=excluded.contents,updated_at=CURRENT_TIMESTAMP").bind(id).bind(bytes.to_vec()).execute(state.store.pool()).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn upload_project_symbol(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let exists = sqlx::query("SELECT 1 FROM projects WHERE project_id=?")
        .bind(project_id)
        .fetch_optional(state.store.pool())
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("Project does not exist"));
    }
    let filename = query
        .get("filename")
        .ok_or_else(|| ApiError::bad_request("filename is required"))?;
    let id = safe_symbol_id(filename)?;
    let bytes = to_bytes(request.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    if bytes.is_empty() {
        return Err(ApiError::bad_request("symbol file is empty"));
    }
    sqlx::query("INSERT INTO controller_symbols(symbol_id,contents) VALUES(?,?) ON CONFLICT(symbol_id) DO UPDATE SET contents=excluded.contents,updated_at=CURRENT_TIMESTAMP")
        .bind(id)
        .bind(bytes.to_vec())
        .execute(state.store.pool())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn delete_symbol(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let r = sqlx::query("DELETE FROM controller_symbols WHERE symbol_id=?")
        .bind(id)
        .execute(state.store.pool())
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found("Symbol does not exist"));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn symbol_dimensions(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let row = sqlx::query("SELECT contents FROM controller_symbols WHERE symbol_id=?")
        .bind(id)
        .fetch_optional(state.store.pool())
        .await?
        .ok_or_else(|| ApiError::not_found("Symbol does not exist"))?;
    let bytes: Vec<u8> = row.get("contents");
    let (width, height) = image_dimensions(&bytes).ok_or_else(|| {
        ApiError::bad_request("symbol dimensions are not supported for this file format")
    })?;
    Ok(Json(json!({"width":width,"height":height})))
}
pub(super) fn content_type(path: &str) -> &'static str {
    if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}
pub(super) fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
        return Some((
            u32::from_be_bytes(data[16..20].try_into().ok()?),
            u32::from_be_bytes(data[20..24].try_into().ok()?),
        ));
    }
    let s = std::str::from_utf8(data).ok()?;
    let tag = s.find("<svg")?;
    let end = s[tag..].find('>')? + tag;
    let root = &s[tag..end];
    let dimension = |key: &str| -> Option<u32> {
        let start = root.find(key)? + key.len();
        let rest = root[start..].trim_start_matches([' ', '=', '"', '\'']);
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    };
    Some((dimension("width")?, dimension("height")?))
}
