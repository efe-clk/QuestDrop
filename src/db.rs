//! Postgres access for Phase 1 (Drop + Freeze).
//! Drop = one transaction: user + project(OPEN) + swap_offer(OPEN) + outbox event.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::validate::ValidatedDrop;

pub const MAX_DROPS_PER_DAY: i64 = 3;

#[derive(Debug)]
pub enum DbError {
    /// Same email with a different handle (or vice versa): identity conflict.
    Conflict(String),
    /// Spec §6: max 3 drops per day. Carries retry-after seconds.
    DailyCap(u64),
    /// Unknown id (taker, offer): caller maps to 404.
    NotFound(String),
    /// Malformed-but-parseable client error: caller maps to 400.
    BadRequest(String),
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
    /// The project's OPEN offer (what POST /v1/swaps takes). None once the
    /// project left the pool, e.g. inside a revive package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer_id: Option<Uuid>,
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

/// Creates project + open offer + outbox event atomically for a known
/// (session) user. The user row is locked first so two concurrent drops
/// cannot both pass the daily cap. Identity (handle/email) comes from the
/// session, never from the request body.
pub async fn create_drop_as(
    pool: &PgPool,
    giver: Uuid,
    v: &ValidatedDrop,
) -> Result<(Uuid, Uuid), DbError> {
    let mut tx = pool.begin().await?;
    if sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(giver)
        .fetch_optional(&mut *tx)
        .await?
        .is_none()
    {
        tx.rollback().await?;
        return Err(DbError::NotFound("unknown user".into()));
    }
    let today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM projects WHERE giver_id = $1 AND created_at > now() - interval '1 day'",
    )
    .bind(giver)
    .fetch_one(&mut *tx)
    .await?;
    if today >= MAX_DROPS_PER_DAY {
        // Rolling 24h window: retry when the oldest counted drop expires,
        // so Retry-After matches the actual quota (not UTC midnight).
        let oldest: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT MIN(created_at) FROM projects WHERE giver_id = $1 AND created_at > now() - interval '1 day'",
        )
        .bind(giver)
        .fetch_one(&mut *tx)
        .await?;
        tx.rollback().await?;
        let retry = oldest
            .map(|t| (86_400 - (Utc::now() - t).num_seconds()).max(1) as u64)
            .unwrap_or(3_600);
        return Err(DbError::DailyCap(retry));
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

/// Column list shared by every PoolRow query (alias p = projects,
/// u = giver, o = lateral OPEN offer).
const POOL_COLS: &str = "p.id, p.title, p.one_liner, p.link_url, p.voice_url, p.skill_needed,
     p.time_bucket::text AS time_bucket, p.energy::text AS energy,
     u.handle AS giver_handle, p.created_at, o.id AS offer_id";
const POOL_FROM: &str = "FROM projects p JOIN users u ON u.id = p.giver_id
     LEFT JOIN LATERAL (
       SELECT id FROM swap_offers so
       WHERE so.project_id = p.id AND so.status = 'OPEN'::offer_status
       ORDER BY so.created_at LIMIT 1
     ) o ON TRUE";

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
    let sql = format!(
        "SELECT {POOL_COLS} {POOL_FROM}
         WHERE p.status = 'OPEN'::project_status AND (p.created_at, p.id) < ($1, $2)
         ORDER BY p.created_at DESC, p.id DESC
         LIMIT $3"
    );
    let rows: Vec<PoolRow> = sqlx::query_as(&sql)
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
        "SELECT p.id::text AS id, o.id::text AS offer_id, p.skill_needed,
                p.time_bucket::text AS time_bucket, p.energy::text AS energy
         FROM projects p
         JOIN swap_offers o ON o.project_id = p.id AND o.status = 'OPEN'::offer_status
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

#[derive(Debug, Serialize, Deserialize)]
pub struct SwapBody {
    pub match_id: uuid::Uuid,
    pub give_offer_id: uuid::Uuid,
    pub take_offer_id: uuid::Uuid,
    pub project_given: uuid::Uuid,
    pub project_taken: uuid::Uuid,
    pub score: f64,
}

struct OfferRow {
    id: uuid::Uuid,
    project_id: uuid::Uuid,
    giver_id: uuid::Uuid,
    status: String,
}

/// Atomic swap: both offers (and their projects) go OPEN -> MATCHED, one
/// match row + one outbox `swap.completed` event. Rows are locked in id
/// order so concurrent swaps cannot deadlock or double-claim.
/// Returns (http_status, body): 201 fresh, 200 idempotent replay.
pub async fn create_swap(
    pool: &PgPool,
    taker: Uuid,
    give_offer: Uuid,
    take_offer: Uuid,
    idem_key: Option<&str>,
) -> Result<(u16, SwapBody), DbError> {
    if give_offer == take_offer {
        return Err(DbError::Conflict("give and take offers must differ".into()));
    }
    if !sqlx::query("SELECT 1 FROM users WHERE id = $1")
        .bind(taker)
        .fetch_optional(pool)
        .await?
        .is_some()
    {
        return Err(DbError::NotFound("unknown taker".into()));
    }

    let mut tx = pool.begin().await?;

    // Idempotency gate INSIDE the tx: the PK serializes concurrent replays.
    // A placeholder row reserves the key; a conflict means replay — but only
    // for the IDENTICAL request (Stripe rule): same key + different params
    // is a client bug and must fail, not replay someone else's result.
    let req_json = serde_json::json!({"taker_id": taker, "give_offer_id": give_offer, "take_offer_id": take_offer});
    if let Some(key) = idem_key {
        let inserted: bool = sqlx::query_scalar(
            "INSERT INTO idempotency_keys (key, taker_id, status_code, body, req)
             VALUES ($1, $2, 0, '{}', $3) ON CONFLICT DO NOTHING RETURNING TRUE",
        )
        .bind(key)
        .bind(taker)
        .bind(sqlx::types::Json(&req_json))
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        if !inserted {
            let row = sqlx::query(
                "SELECT body, req FROM idempotency_keys WHERE key = $1 AND taker_id = $2",
            )
            .bind(key)
            .bind(taker)
            .fetch_one(&mut *tx)
            .await?;
            tx.rollback().await?;
            let stored_req: sqlx::types::Json<serde_json::Value> = row.try_get("req")?;
            if stored_req.0 != req_json {
                return Err(DbError::BadRequest(
                    "idempotency-key already used with different parameters".into(),
                ));
            }
            let stored: sqlx::types::Json<serde_json::Value> = row.try_get("body")?;
            let body: SwapBody = serde_json::from_value(stored.0).map_err(|e| {
                DbError::Db(sqlx::Error::Decode(
                    format!("stored swap body corrupt: {e}").into(),
                ))
            })?;
            // Replay answers 200 (Stripe convention) with the identical body;
            // the stored 201 stays as the record of the original execution.
            return Ok((200, body));
        }
    }

    let rows: Vec<OfferRow> = sqlx::query(
        "SELECT id, project_id, giver_id, status::text AS status FROM swap_offers
         WHERE id = ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind([give_offer, take_offer])
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| {
        Ok::<_, sqlx::Error>(OfferRow {
            id: r.try_get("id")?,
            project_id: r.try_get("project_id")?,
            giver_id: r.try_get("giver_id")?,
            status: r.try_get("status")?,
        })
    })
    .collect::<Result<_, _>>()?;
    if rows.len() != 2 {
        tx.rollback().await?;
        return Err(DbError::NotFound("offer not found".into()));
    }
    let give = rows.iter().find(|o| o.id == give_offer);
    let take = rows.iter().find(|o| o.id == take_offer);
    let (Some(give), Some(take)) = (give, take) else {
        tx.rollback().await?;
        return Err(DbError::NotFound("offer not found".into()));
    };
    if give.status != "OPEN" || take.status != "OPEN" {
        tx.rollback().await?;
        return Err(DbError::Conflict("offer is no longer open".into()));
    }
    if give.giver_id != taker {
        tx.rollback().await?;
        return Err(DbError::Conflict("give offer is not yours".into()));
    }
    if take.giver_id == taker {
        tx.rollback().await?;
        return Err(DbError::Conflict("cannot take your own project".into()));
    }

    // Deterministic fit between taker skills and the taken project.
    let taker_skills: (Vec<String>, Vec<String>) =
        sqlx::query_as("SELECT can_do, looking_for FROM users WHERE id = $1")
            .bind(taker)
            .fetch_one(&mut *tx)
            .await?;
    let taken_skills: Vec<String> =
        sqlx::query_scalar("SELECT skill_needed FROM projects WHERE id = $1")
            .bind(take.project_id)
            .fetch_one(&mut *tx)
            .await?;
    let score = crate::matcher::fit_score(
        &crate::matcher::QuestUser {
            id: String::new(),
            can_do: taker_skills.0,
            looking_for: taker_skills.1,
        },
        &crate::matcher::PoolItem {
            id: String::new(),
            offer_id: String::new(),
            skill_needed: taken_skills,
            time_bucket: String::new(),
            energy: String::new(),
        },
    );

    sqlx::query("UPDATE swap_offers SET status = 'MATCHED' WHERE id = ANY($1)")
        .bind([give_offer, take_offer])
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE projects SET status = 'MATCHED' WHERE id = ANY($1)")
        .bind([give.project_id, take.project_id])
        .execute(&mut *tx)
        .await?;
    let match_id: Uuid = sqlx::query_scalar(
        "INSERT INTO matches (taker_id, given_id, taken_id, score, reason)
         VALUES ($1, $2, $3, $4, 'manual swap') RETURNING id",
    )
    .bind(taker)
    .bind(give.project_id)
    .bind(take.project_id)
    .bind(score)
    .fetch_one(&mut *tx)
    .await?;
    let body = SwapBody {
        match_id,
        give_offer_id: give_offer,
        take_offer_id: take_offer,
        project_given: give.project_id,
        project_taken: take.project_id,
        score,
    };
    let payload = serde_json::json!({
        "match_id": match_id,
        "give_offer_id": give_offer,
        "take_offer_id": take_offer,
        "taker_id": taker,
    });
    sqlx::query("INSERT INTO outbox_events (type, payload) VALUES ('swap.completed', $1)")
        .bind(sqlx::types::Json(payload))
        .execute(&mut *tx)
        .await?;
    if let Some(key) = idem_key {
        sqlx::query(
            "UPDATE idempotency_keys SET status_code = 201, body = $1 WHERE key = $2 AND taker_id = $3",
        )
        .bind(sqlx::types::Json(&body))
        .bind(key)
        .bind(taker)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok((201, body))
}

/// Reports a project. Third distinct report auto-hides it (spec §6):
/// project + its open offers go OPEN -> CLOSED in the same transaction.
/// Returns (report_id, hidden_now).
pub async fn create_report(
    pool: &PgPool,
    reporter: Uuid,
    project: Uuid,
    reason: &str,
) -> Result<(Uuid, bool), DbError> {
    let mut tx = pool.begin().await?;
    if sqlx::query("SELECT 1 FROM users WHERE id = $1")
        .bind(reporter)
        .fetch_optional(&mut *tx)
        .await?
        .is_none()
    {
        tx.rollback().await?;
        return Err(DbError::NotFound("unknown reporter".into()));
    }
    let giver: Option<Uuid> = sqlx::query_scalar("SELECT giver_id FROM projects WHERE id = $1")
        .bind(project)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(giver) = giver else {
        tx.rollback().await?;
        return Err(DbError::NotFound("unknown project".into()));
    };
    if giver == reporter {
        tx.rollback().await?;
        return Err(DbError::Conflict("cannot report your own project".into()));
    }
    let report_id: Uuid = sqlx::query_scalar(
        "INSERT INTO reports (project_id, reporter_id, reason) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(project)
    .bind(reporter)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23505") => {
            DbError::Conflict("already reported".into())
        }
        _ => DbError::Db(e),
    })?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE project_id = $1")
        .bind(project)
        .fetch_one(&mut *tx)
        .await?;
    if n >= 3 {
        sqlx::query(
            "UPDATE projects SET status = 'CLOSED' WHERE id = $1 AND status = 'OPEN'::project_status",
        )
        .bind(project)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE swap_offers SET status = 'CLOSED' WHERE project_id = $1 AND status = 'OPEN'::offer_status",
        )
        .bind(project)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    // Re-read after commit: a concurrent reporter may have closed the
    // project between our COUNT and commit, so report actual final state.
    let status: String = sqlx::query_scalar("SELECT status::text FROM projects WHERE id = $1")
        .bind(project)
        .fetch_one(pool)
        .await?;
    Ok((report_id, status != "OPEN"))
}

#[derive(Debug, Serialize)]
pub struct FirstTask {
    pub title: String,
    pub minutes: u8,
    pub steps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Revive {
    pub match_id: Uuid,
    pub matched_at: DateTime<Utc>,
    pub project: PoolRow,
    pub first_task: FirstTask,
}

/// First 2-minute task for a revived quest: open, listen, smallest step.
pub fn first_task_for(project: &PoolRow) -> FirstTask {
    let hands_on = project
        .skill_needed
        .first()
        .map(|s| format!("Reproduce the smallest '{s}' piece locally"))
        .unwrap_or_else(|| "Reproduce the smallest piece locally".into());
    FirstTask {
        title: format!("First 2 minutes with '{}'", project.title),
        minutes: 2,
        steps: vec![
            "Open the project link and skim for 30 seconds".into(),
            "Listen to the voice note end to end".into(),
            hands_on,
            "Write down the single next step".into(),
        ],
    }
}

/// All completed swaps for a taker, newest first — each with its freeze
/// package + first task. Latest-only would strand older quests.
pub async fn all_revives(pool: &PgPool, taker: Uuid) -> Result<Vec<Revive>, DbError> {
    let rows = sqlx::query(
        "SELECT m.id AS match_id, m.created_at AS matched_at,
                p.id, p.title, p.one_liner, p.link_url, p.voice_url, p.skill_needed,
                p.time_bucket::text AS time_bucket, p.energy::text AS energy,
                u.handle AS giver_handle, p.created_at
         FROM matches m
         JOIN projects p ON p.id = m.taken_id
         JOIN users u ON u.id = p.giver_id
         WHERE m.taker_id = $1
         ORDER BY m.created_at DESC, m.id DESC",
    )
    .bind(taker)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let project = PoolRow {
            id: r.try_get("id")?,
            offer_id: None,
            title: r.try_get("title")?,
            one_liner: r.try_get("one_liner")?,
            link_url: r.try_get("link_url")?,
            voice_url: r.try_get("voice_url")?,
            skill_needed: r.try_get("skill_needed")?,
            time_bucket: r.try_get("time_bucket")?,
            energy: r.try_get("energy")?,
            giver_handle: r.try_get("giver_handle")?,
            created_at: r.try_get("created_at")?,
        };
        let task = first_task_for(&project);
        out.push(Revive {
            match_id: r.try_get("match_id")?,
            matched_at: r.try_get("matched_at")?,
            project,
            first_task: task,
        });
    }
    Ok(out)
}

/// Session user's identity + skill profile.
#[derive(Debug, Clone, Serialize)]
pub struct UserInfo {
    pub id: Uuid,
    pub handle: String,
    pub email: String,
    pub can_do: Vec<String>,
    pub looking_for: Vec<String>,
}

pub async fn user_info(pool: &PgPool, id: Uuid) -> Result<Option<UserInfo>, DbError> {
    let row = sqlx::query("SELECT id, handle, email, can_do, looking_for FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let Some(r) = row else { return Ok(None) };
    Ok(Some(UserInfo {
        id: r.try_get("id")?,
        handle: r.try_get("handle")?,
        email: r.try_get("email")?,
        can_do: r.try_get("can_do")?,
        looking_for: r.try_get("looking_for")?,
    }))
}

/// Updates the session user's skills (identity is never edited here).
pub async fn set_skills(
    pool: &PgPool,
    id: Uuid,
    can_do: &[String],
    looking_for: &[String],
) -> Result<(), DbError> {
    sqlx::query("UPDATE users SET can_do = $1, looking_for = $2 WHERE id = $3")
        .bind(can_do)
        .bind(looking_for)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// One project with its lifecycle status and owner (for /q/{id} and the
/// take screen). Includes hidden/matched ones so callers can explain why
/// a quest is no longer available.
#[derive(Debug, Serialize)]
pub struct ProjectDetail {
    #[serde(flatten)]
    pub row: PoolRow,
    pub status: String,
    #[serde(skip)]
    pub giver_id: Uuid,
}

pub async fn get_project(pool: &PgPool, id: Uuid) -> Result<Option<ProjectDetail>, DbError> {
    let sql = format!(
        "SELECT {POOL_COLS}, p.status::text AS status, p.giver_id {POOL_FROM} WHERE p.id = $1"
    );
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    let Some(r) = row else { return Ok(None) };
    Ok(Some(ProjectDetail {
        row: pool_row_from(&r)?,
        status: r.try_get("status")?,
        giver_id: r.try_get("giver_id")?,
    }))
}

/// Project behind an offer id (any status).
pub async fn project_for_offer(
    pool: &PgPool,
    offer: Uuid,
) -> Result<Option<ProjectDetail>, DbError> {
    let pid: Option<Uuid> = sqlx::query_scalar("SELECT project_id FROM swap_offers WHERE id = $1")
        .bind(offer)
        .fetch_optional(pool)
        .await?;
    match pid {
        Some(p) => get_project(pool, p).await,
        None => Ok(None),
    }
}

/// Full rows for ranked ids, returned in the SAME order as `ids`
/// (missing ids are skipped).
pub async fn projects_by_ids(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<PoolRow>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT {POOL_COLS} {POOL_FROM} WHERE p.id = ANY($1)");
    let rows: Vec<PoolRow> = sqlx::query_as(&sql).bind(ids).fetch_all(pool).await?;
    let mut by_id: std::collections::HashMap<Uuid, PoolRow> =
        rows.into_iter().map(|r| (r.id, r)).collect();
    Ok(ids.iter().filter_map(|i| by_id.remove(i)).collect())
}

/// A quest the user dropped, with where it is now.
#[derive(Debug, Serialize)]
pub struct MyQuest {
    #[serde(flatten)]
    pub row: PoolRow,
    /// OPEN (in pool) | MATCHED (passed on) | CLOSED (hidden by reports)
    pub status: String,
}

/// Everything the user dropped, newest first. OPEN ones carry `offer_id`
/// (what the take screen offers in exchange).
pub async fn my_quests(pool: &PgPool, user: Uuid) -> Result<Vec<MyQuest>, DbError> {
    let sql = format!(
        "SELECT {POOL_COLS}, p.status::text AS status {POOL_FROM}
         WHERE p.giver_id = $1
         ORDER BY p.created_at DESC, p.id DESC LIMIT 200"
    );
    let rows = sqlx::query(&sql).bind(user).fetch_all(pool).await?;
    rows.iter()
        .map(|r| {
            Ok(MyQuest {
                row: pool_row_from(r)?,
                status: r.try_get("status")?,
            })
        })
        .collect()
}

fn pool_row_from(r: &sqlx::postgres::PgRow) -> Result<PoolRow, DbError> {
    Ok(PoolRow {
        id: r.try_get("id")?,
        title: r.try_get("title")?,
        one_liner: r.try_get("one_liner")?,
        link_url: r.try_get("link_url")?,
        voice_url: r.try_get("voice_url")?,
        skill_needed: r.try_get("skill_needed")?,
        time_bucket: r.try_get("time_bucket")?,
        energy: r.try_get("energy")?,
        giver_handle: r.try_get("giver_handle")?,
        created_at: r.try_get("created_at")?,
        offer_id: r.try_get("offer_id")?,
    })
}

/// Projects this user already reported (the card hides its report menu).
pub async fn reported_ids(pool: &PgPool, reporter: Uuid) -> Result<Vec<Uuid>, DbError> {
    Ok(
        sqlx::query_scalar("SELECT project_id FROM reports WHERE reporter_id = $1")
            .bind(reporter)
            .fetch_all(pool)
            .await?,
    )
}

/// Seconds until this user may drop again, or None if under the daily cap.
/// Read-only twin of the check inside `create_drop_as` (which stays the
/// authority); lets the form say so before the user fills it in.
pub async fn daily_cap_retry(pool: &PgPool, user: Uuid) -> Result<Option<u64>, DbError> {
    let (n, oldest): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT COUNT(*), MIN(created_at) FROM projects
         WHERE giver_id = $1 AND created_at > now() - interval '1 day'",
    )
    .bind(user)
    .fetch_one(pool)
    .await?;
    if n < MAX_DROPS_PER_DAY {
        return Ok(None);
    }
    Ok(Some(
        oldest
            .map(|t| (86_400 - (Utc::now() - t).num_seconds()).max(1) as u64)
            .unwrap_or(3_600),
    ))
}

/// Seconds until this email may get another login link (0 = now).
pub async fn login_cooldown(pool: &PgPool, email: &str, window_secs: i64) -> Result<i64, DbError> {
    let last: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM magic_tokens WHERE email = $1")
            .bind(email)
            .fetch_one(pool)
            .await?;
    Ok(last
        .map(|t| (window_secs - (Utc::now() - t).num_seconds()).max(0))
        .unwrap_or(0))
}

/// One of the caller's own offers with its project: what a client lists
/// when the taker picks which quest to give in a swap.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MyOffer {
    pub offer_id: Uuid,
    /// OPEN | MATCHED | CLOSED
    pub offer_status: String,
    pub project_id: Uuid,
    pub title: String,
    pub one_liner: String,
    pub link_url: String,
    pub voice_url: String,
    pub skill_needed: Vec<String>,
    pub time_bucket: String,
    pub energy: String,
    pub created_at: DateTime<Utc>,
}

/// The giver's offers, newest first (cap 200), optionally filtered by
/// status (already normalized to OPEN | MATCHED | CLOSED by the caller).
pub async fn my_offers(
    pool: &PgPool,
    giver: Uuid,
    status: Option<&str>,
) -> Result<Vec<MyOffer>, DbError> {
    Ok(sqlx::query_as::<_, MyOffer>(
        "SELECT o.id AS offer_id, o.status::text AS offer_status, p.id AS project_id,
                p.title, p.one_liner, p.link_url, p.voice_url, p.skill_needed,
                p.time_bucket::text AS time_bucket, p.energy::text AS energy, o.created_at
         FROM swap_offers o JOIN projects p ON p.id = o.project_id
         WHERE o.giver_id = $1 AND ($2::text IS NULL OR o.status::text = $2)
         ORDER BY o.created_at DESC, o.id DESC LIMIT 200",
    )
    .bind(giver)
    .bind(status)
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> PoolRow {
        PoolRow {
            id: Uuid::nil(),
            offer_id: None,
            title: "Demo".into(),
            one_liner: "x".into(),
            link_url: "https://x.y".into(),
            voice_url: "/voice/a.mp3".into(),
            skill_needed: vec!["rust".into()],
            time_bucket: "S".into(),
            energy: "LOW".into(),
            giver_handle: "g".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn first_task_is_two_minutes_and_skill_aware() {
        let t = first_task_for(&row());
        assert_eq!(t.minutes, 2);
        assert_eq!(t.steps.len(), 4);
        assert!(t.steps[2].contains("rust"));
        let mut bare = row();
        bare.skill_needed.clear();
        assert!(first_task_for(&bare).steps[2].contains("smallest piece"));
    }
}
