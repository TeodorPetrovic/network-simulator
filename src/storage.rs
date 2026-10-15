use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sqlx::{
    Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use tokio::sync::broadcast;

const EVENT_CHANNEL_CAPACITY: usize = 2_048;

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    events: broadcast::Sender<ProjectEvent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectEvent {
    pub project_id: String,
    pub sequence: i64,
    pub action: String,
    pub event: Value,
}

impl Store {
    pub async fn connect(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;

        sqlx::migrate!().run(&pool).await?;

        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Ok(Self { pool, events })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProjectEvent> {
        self.events.subscribe()
    }

    pub fn publish(&self, event: ProjectEvent) {
        let _ = self.events.send(event);
    }

    pub async fn append_event(
        tx: &mut Transaction<'_, Sqlite>,
        project_id: &str,
        action: &str,
        event: &Value,
    ) -> Result<ProjectEvent, sqlx::Error> {
        sqlx::query("UPDATE projects SET revision = revision + 1 WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
        let event_json = serde_json::to_string(event).expect("JSON values always serialize");
        let result = sqlx::query(
            "INSERT INTO project_events (project_id, action, event_json) VALUES (?, ?, ?)",
        )
        .bind(project_id)
        .bind(action)
        .bind(event_json)
        .execute(&mut **tx)
        .await?;

        Ok(ProjectEvent {
            project_id: project_id.to_owned(),
            sequence: result.last_insert_rowid(),
            action: action.to_owned(),
            event: event.clone(),
        })
    }

    pub async fn events_after(
        &self,
        project_id: &str,
        sequence: i64,
        limit: i64,
    ) -> Result<Vec<ProjectEvent>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (i64, String, String)>(
            "SELECT sequence, action, event_json FROM project_events
             WHERE project_id = ? AND sequence > ? ORDER BY sequence LIMIT ?",
        )
        .bind(project_id)
        .bind(sequence)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(sequence, action, event_json)| {
                let event = serde_json::from_str(&event_json)
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
                Ok(ProjectEvent {
                    project_id: project_id.to_owned(),
                    sequence,
                    action,
                    event,
                })
            })
            .collect()
    }
}
