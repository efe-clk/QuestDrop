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
    /// Internal: unique race lost mid-transaction — caller retries with a
    /// FRESH transaction (a 23505 poisons the current one: Postgres aborts
    /// it, so re-querying inside it always fails).
    RetryTx,
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
            return Err(DbError::Conflict(
                "email is already registered with a different handle".into(),
            ));
        }
        return Ok(id);
    }
    if sqlx::query("SELECT 1 FROM users WHERE handle = $1")
        .bind(handle)
        .fetch_optional(&mut **tx)
        .await?
        .is_some()
    {
        return Err(DbError::Conflict("handle is already taken".into()));
    }
    sqlx::query_scalar("INSERT INTO users (handle, email) VALUES ($1, $2) RETURNING id")
        .bind(handle)
        .bind(email)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| match &e {
            // Lost an insert race with a concurrent first-drop. The current
            // transaction is now aborted — signal for a fresh retry.
            sqlx::Error::Database(d) if d.code().as_deref() == Some("23505") => DbError::RetryTx,
            _ => DbError::Db(e),
        })
}

/// Creates project + open offer + outbox event atomically.
/// The user row is locked first so two concurrent drops cannot both
/// pass the daily cap. Insert races retry with a FRESH transaction.
pub async fn create_drop(pool: &PgPool, v: &ValidatedDrop) -> Result<(Uuid, Uuid), DbError> {
    for _ in 0..3 {
        let mut tx = pool.begin().await?;
        let giver = match find_or_create_user(&mut tx, &v.handle, &v.email).await {
            Ok(id) => id,
            Err(DbError::RetryTx) => continue, // tx drops uncommitted = rollback
            Err(e) => return Err(e),
        };

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
        return Ok((project_id, offer_id));
    }
    Err(DbError::Conflict(
        "concurrent registration, please retry".into(),
    ))
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
            // Corrupt row (impossible via NOT NULL schema): restart from top
            // rather than panicking the request.
            .and_then(|r| Some((r.try_get("created_at").ok()?, r.try_get("id").ok()?))),
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
    let next = if more {
        page.last().map(|r| r.id)
    } else {
        None
    };
    Ok((page, next))
}

pub async fn count_open(pool: &PgPool) -> Result<i64, DbError> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE status = 'OPEN'::project_status")
            .fetch_one(pool)
            .await?,
    )
}

/// Skills profile for matching. None = unknown user (caller maps to 404).
pub async fn load_user(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<crate::matcher::QuestUser>, DbError> {
    Ok(sqlx::query_as::<_, crate::matcher::QuestUser>(
        "SELECT id::text AS id, can_do, looking_for FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// OPEN pool minus the requester's own projects, newest first (cap 100).
pub async fn match_candidates(
    pool: &PgPool,
    giver: Uuid,
) -> Result<Vec<crate::matcher::PoolItem>, DbError> {
    Ok(sqlx::query_as::<_, crate::matcher::PoolItem>(
        "SELECT p.id::text AS id, p.skill_needed,
                p.time_bucket::text AS time_bucket, p.energy::text AS energy
         FROM projects p
         WHERE p.status = 'OPEN'::project_status AND p.giver_id != $1
         ORDER BY p.created_at DESC LIMIT 100",
    )
    .bind(giver)
    .fetch_all(pool)
    .await?)
}

/// Creates or updates a skill profile (used by matching).
/// Same identity rule as drops: an email keeps its handle, or 409.
pub async fn upsert_profile(
    pool: &PgPool,
    v: &crate::validate::ValidatedProfile,
) -> Result<Uuid, DbError> {
    let row = sqlx::query("SELECT id, handle FROM users WHERE email = $1")
        .bind(&v.email)
        .fetch_optional(pool)
        .await?;
    if let Some(r) = row {
        let id: Uuid = r.try_get("id")?;
        let existing: String = r.try_get("handle")?;
        if existing != v.handle {
            return Err(DbError::Conflict(
                "email is already registered with a different handle".into(),
            ));
        }
        sqlx::query("UPDATE users SET can_do = $1, looking_for = $2 WHERE id = $3")
            .bind(&v.can_do)
            .bind(&v.looking_for)
            .bind(id)
            .execute(pool)
            .await?;
        return Ok(id);
    }
    sqlx::query_scalar(
        "INSERT INTO users (handle, email, can_do, looking_for) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&v.handle)
    .bind(&v.email)
    .bind(&v.can_do)
    .bind(&v.looking_for)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23505") => {
            DbError::Conflict("handle or email is already taken".into())
        }
        _ => DbError::Db(e),
    })
}
