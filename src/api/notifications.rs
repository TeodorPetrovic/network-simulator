use super::*;

pub(super) async fn project_notifications(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Query(query): Query<NotificationQuery>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id.clone());
        websocket_session(socket, state, project_id, query.after).await;
    }))
}

#[derive(Default, serde::Deserialize)]
pub(super) struct NotificationQuery {
    after: Option<i64>,
}

pub(super) async fn websocket_session(
    socket: WebSocket,
    state: AppState,
    project_id: String,
    after: Option<i64>,
) {
    let mut receiver = state.store.subscribe();
    let (mut sink, mut stream) = socket.split();
    let mut cursor = match after {
        Some(cursor) => cursor,
        None => match latest_project_event(state.store.pool(), &project_id).await {
            Ok(cursor) => cursor,
            Err(error) => {
                tracing::error!(%error, %project_id, "could not initialize project event stream");
                return;
            }
        },
    };

    if after.is_some()
        && !send_events_after(&mut sink, &state.store, &project_id, &mut cursor).await
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            event = receiver.recv() => {
                match event {
                    Ok(event) if event.project_id == project_id && event.sequence > cursor => {
                        if !send_event(&mut sink, &event).await {
                            break;
                        }
                        cursor = event.sequence;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(%project_id, skipped, "project event subscriber lagged; replaying from storage");
                        if !send_events_after(&mut sink, &state.store, &project_id, &mut cursor).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

pub(super) async fn send_events_after(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    store: &Store,
    project_id: &str,
    cursor: &mut i64,
) -> bool {
    loop {
        let events = match store
            .events_after(project_id, *cursor, EVENT_PAGE_SIZE)
            .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::error!(%error, %project_id, "could not replay project events");
                return false;
            }
        };
        if events.is_empty() {
            return true;
        }
        let count = events.len();
        for event in events {
            if !send_event(sink, &event).await {
                return false;
            }
            *cursor = event.sequence;
        }
        if count < EVENT_PAGE_SIZE as usize {
            return true;
        }
    }
}

pub(super) async fn send_event(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &ProjectEvent,
) -> bool {
    match serde_json::to_string(event) {
        Ok(text) => sink.send(Message::Text(text.into())).await.is_ok(),
        Err(error) => {
            tracing::error!(%error, "could not encode project notification");
            false
        }
    }
}

pub(super) async fn latest_project_event(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) FROM project_events WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(pool)
        .await
}
