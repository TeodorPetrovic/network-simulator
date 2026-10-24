use super::*;

pub(super) async fn me(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    get_user_value(state.store.pool(), &principal.user_id)
        .await
        .map(Json)
}

pub(super) async fn update_me(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    update_user_row(state.store.pool(), &principal.user_id, &body, true).await?;
    get_user_value(state.store.pool(), &principal.user_id)
        .await
        .map(Json)
}

