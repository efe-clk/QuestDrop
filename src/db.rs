//! Postgres access for Phase 1 (Drop + Freeze).
//! Drop = one transaction: user + project(OPEN) + swap_offer(OPEN) + outbox event.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::validate::ValidatedDrop;

pub const MAX_DROPS_PER_DAY: i64 = 3;

#[derive(Debug)]
pub enum DbError {
    /// Same email with a different handle (or vice versa): identity conflict.
    Conflict(String),
    /// Spec §6: max 3 drops per day.
    DailyCap,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> Self {
        DbError::Db(e)
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PoolRow {
    pub id: Uuid,
    pub title: String,
    pub one_liner: String,
    pub link_url: String,
    pub voice_url: String,
    pub skill_needed: Vec<String>,
    pub time_bucket: String,
    pub energy: String,
    pub giver_handle: String,
    pub created_at: DateTime<Utc>,
}

async fn find_or_create_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    handle: &str,
    email: &str,
) -> Result<Uuid, DbError> {
    if let Some(row) = sqlx::query("SELECT id, handle FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(&mut **tx)
        .await?
    {
        let id: Uuid = row.try_get("id")?;
        let existing: String = row.try_get("handle")?;
        if existing != handle {
            return Err(DbError::Conflict(format!(
                "email is already registered with a different handle"
            )));
        }
        return Ok(id);
    }
    if sqlx::query("SELECT 1 FROM users WHERE handle = $1")
        .bind(handle)
        .fetch_optional(&mut **tx)
        .await?
        .is_some()
    {
        return Err(DbError::Conflict(format!("handle is already taken")));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (handle, email) VALUES ($1, $2) RETURNING id",
    )
    .bind(handle)
    .bind(email)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| match &e {
        // Two concurrent first-drops can both pass the SELECT checks above;
        // the UNIQUE constraint arbitrates — report 409, not 503.
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23505") => {
            DbError::Conflict("handle or email is already taken".into())
        }
        _ => DbError::Db(e),
    })?;
    Ok(id)
}

/// Creates project + open offer + outbox event atomically.
/// The user row is locked first so two concurrent drops cannot both
/// pass the daily cap.
pub async fn create_drop(pool: &PgPool, v: &ValidatedDrop) -> Result<(Uuid, Uuid), DbError> {
    let mut tx = pool.begin().await?;
    let giver = find_or_create_user(&mut tx, &v.handle, &v.email).await?;

    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(giver)
        .fetch_one(&mut *tx)
        .await?;
    let today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM projects WHERE giver_id = $1 AND created_at > now() - interval '1 day'",
    )
    .bind(giver)
    .fetch_one(&mut *tx)
    .await?;
    if today >= MAX_DROPS_PER_DAY {
        tx.rollback().await?;
        return Err(DbError::DailyCap);
    }

    let project_id: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (title, one_liner, link_url, voice_url, skill_needed, time_bucket, energy, giver_id)
         VALUES ($1, $2, $3, $4, $5, $6::time_bucket, $7::energy, $8) RETURNING id",
    )
    .bind(&v.title)
    .bind(&v.one_liner)
    .bind(&v.link_url)
    .bind(&v.voice_url)
    .bind(&v.skill_needed)
    .bind(&v.time_bucket)
    .bind(&v.energy)
    .bind(giver)
    .fetch_one(&mut *tx)
    .await?;

    let offer_id: Uuid = sqlx::query_scalar(
        "INSERT INTO swap_offers (project_id, giver_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(project_id)
    .bind(giver)
    .fetch_one(&mut *tx)
    .await?;

    let payload = serde_json::json!({
        "project_id": project_id,
        "offer_id": offer_id,
        "giver_id": giver,
    });
    sqlx::query("INSERT INTO outbox_events (type, payload) VALUES ('swap.created', $1)")
        .bind(sqlx::types::Json(payload))
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok((project_id, offer_id))
}

/// Cursor-keyed pool page (newest first). Unknown cursor restarts from the top.
pub async fn list_pool(
    pool: &PgPool,
    cursor: Option<Uuid>,
    limit: i64,
) -> Result<(Vec<PoolRow>, Option<Uuid>), DbError> {
    let bound: Option<(DateTime<Utc>, Uuid)> = match cursor {
        Some(id) => sqlx::query("SELECT created_at, id FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .map(|r| (r.try_get("created_at").unwrap(), r.try_get("id").unwrap())),
        None => None,
    };
    let (t, i) = bound.unwrap_or((Utc::now(), Uuid::max()));
    let rows: Vec<PoolRow> = sqlx::query_as(
        "SELECT p.id, p.title, p.one_liner, p.link_url, p.voice_url, p.skill_needed,
                p.time_bucket::text AS time_bucket, p.energy::text AS energy,
                u.handle AS giver_handle, p.created_at
         FROM projects p JOIN users u ON u.id = p.giver_id
         WHERE p.status = 'OPEN'::project_status AND (p.created_at, p.id) < ($1, $2)
         ORDER BY p.created_at DESC, p.id DESC
         LIMIT $3",
    )
    .bind(t)
    .bind(i)
    .bind(limit + 1)
    .fetch_all(pool)
    .await?;
    let more = rows.len() as i64 > limit;
    let mut page = rows;
    if more {
        page.pop();
    }
    let next = if more { page.last().map(|r| r.id) } else { None };
    Ok((page, next))
}

pub async fn count_open(pool: &PgPool) -> Result<i64, DbError> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM projects WHERE status = 'OPEN'::project_status",
    )
    .fetch_one(pool)
    .await?)
}
