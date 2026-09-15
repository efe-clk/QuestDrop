//! Outbox publisher (B phase, transport half).
//! Polls `outbox_events` for unprocessed rows, delivers each through the
//! configured [`Sink`](crate::notify::Sink), then marks it processed — all
//! inside one transaction per event, so a crash replays instead of losing.

use sqlx::{PgPool, Row};

use crate::notify::{OutboxEvent, Sink};

/// Deliver one batch (max 20). Returns the number delivered.
pub async fn run_once(pool: &PgPool, sink: &dyn Sink) -> Result<usize, sqlx::Error> {
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM outbox_events WHERE NOT processed ORDER BY created_at, id LIMIT 20",
    )
    .fetch_all(pool)
    .await?;
    let mut done = 0;
    for id in ids {
        let mut tx = pool.begin().await?;
        let row = sqlx::query(
            "SELECT id, type, payload FROM outbox_events WHERE id = $1 FOR UPDATE SKIP LOCKED",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(r) = row else { continue };
        // Skip rows another publisher already finished between our scan and lock.
        let processed: bool =
            sqlx::query_scalar("SELECT processed FROM outbox_events WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if processed {
            tx.rollback().await?;
            continue;
        }
        let event = OutboxEvent {
            id: r.try_get("id")?,
            kind: r.try_get("type")?,
            payload: r
                .try_get::<sqlx::types::Json<serde_json::Value>, _>("payload")?
                .0,
        };
        match sink.send(&event).await {
            Ok(()) => {
                sqlx::query("UPDATE outbox_events SET processed = TRUE WHERE id = $1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                done += 1;
            }
            Err(e) => {
                // Leave unprocessed for the next poll; the error is logged.
                tracing::warn!(event_id = %id, error = %e, "sink delivery failed");
                tx.rollback().await?;
            }
        }
    }
    Ok(done)
}
