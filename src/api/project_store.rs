use super::*;

pub(super) async fn fetch_project(pool: &SqlitePool, project_id: &str) -> Result<Value, ApiError> {
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(project_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    decode_json(row.get::<String, _>("project_json"))
}

pub(super) async fn ensure_project_exists_pool(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<(), ApiError> {
    let exists =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM projects WHERE project_id = ?)")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    if exists == 0 {
        return Err(ApiError::not_found(format!(
            "Project {project_id} does not exist"
        )));
    }
    Ok(())
}

pub(super) async fn ensure_project_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
) -> Result<(), ApiError> {
    let exists =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM projects WHERE project_id = ?)")
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await?;
    if exists == 0 {
        return Err(ApiError::not_found(format!(
            "Project {project_id} does not exist"
        )));
    }
    Ok(())
}
