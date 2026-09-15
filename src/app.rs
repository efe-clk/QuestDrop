use std::{net::IpAddr, net::SocketAddr, sync::Arc, time::Duration};

use askama::Template;
use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer, timeout::TimeoutLayer};
use tracing::warn;

use crate::{
    db::{self, DbError},
    matcher::{Matcher, RuleMatcher},
    ratelimit::RateLimiter,
    validate::{validate_drop, validate_profile, validate_report, RawDrop, RawProfile, RawReport},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: Option<sqlx::PgPool>,
    pub voice_dir: std::path::PathBuf,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    pool_count: i64,
}

#[derive(Serialize)]
pub struct Problem {
    #[serde(rename = "type")]
    type_: &'static str,
    title: &'static str,
    status: u16,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<String>>,
}

pub fn problem(status: StatusCode, detail: impl Into<String>) -> (StatusCode, Json<Problem>) {
    (
        status,
        Json(Problem {
            type_: "about:blank",
            title: status.canonical_reason().unwrap_or("Error"),
            status: status.as_u16(),
            detail: detail.into(),
            errors: None,
        }),
    )
}

fn problem_errors(
    status: StatusCode,
    detail: impl Into<String>,
    errors: Vec<String>,
) -> (StatusCode, Json<Problem>) {
    (
        status,
        Json(Problem {
            type_: "about:blank",
            title: status.canonical_reason().unwrap_or("Error"),
            status: status.as_u16(),
            detail: detail.into(),
            errors: Some(errors),
        }),
    )
}

fn db_error(e: DbError) -> Response {
    match e {
        DbError::Conflict(d) => problem(StatusCode::CONFLICT, d).into_response(),
        DbError::NotFound(d) => problem(StatusCode::NOT_FOUND, d).into_response(),
        DbError::DailyCap(retry) => {
            let (status, body) = problem(
                StatusCode::TOO_MANY_REQUESTS,
                format!("max {} drops per day", db::MAX_DROPS_PER_DAY),
            );
            let mut res = (status, body).into_response();
            if let Ok(v) = HeaderValue::from_str(&retry.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
            res
        }
        DbError::Db(inner) => {
            // Never leak driver internals to clients; full error goes to logs.
            tracing::error!(error = %inner, "database failure");
            problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response()
        }
        // Only reachable if a code path forgets to consume it (create_drop
        // always retries internally); fail safe as a transient error.
        DbError::RetryTx => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "concurrent write, please retry",
        )
        .into_response(),
    }
}

async fn index(State(s): State<AppState>) -> Response {
    let count = match &s.pool {
        Some(p) => db::count_open(p).await.unwrap_or(0),
        None => 0,
    };
    match (IndexTemplate { pool_count: count }).render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template render failed");
            problem(StatusCode::INTERNAL_SERVER_ERROR, "render failed").into_response()
        }
    }
}

async fn health(State(s): State<AppState>) -> impl IntoResponse {
    let db = match &s.pool {
        Some(p) => match sqlx::query("SELECT 1 AS one").fetch_one(p).await {
            Ok(_) => "up",
            Err(_) => "down",
        },
        None => "down",
    };
    Json(serde_json::json!({ "status": "ok", "db": db }))
}

#[derive(Deserialize)]
struct PoolQuery {
    cursor: Option<uuid::Uuid>,
    limit: Option<i64>,
}

async fn list_projects(
    State(s): State<AppState>,
    q: Result<Query<PoolQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    let q = match q {
        Ok(Query(q)) => q,
        Err(e) => {
            return problem(StatusCode::BAD_REQUEST, format!("invalid query: {e}")).into_response()
        }
    };
    let limit = q.limit.unwrap_or(20).clamp(1, 50);
    match db::list_pool(pool, q.cursor, limit).await {
        Ok((items, next_cursor)) => Json(serde_json::json!({
            "pool": items,
            "next_cursor": next_cursor,
        }))
        .into_response(),
        Err(e) => db_error(e),
    }
}

#[derive(Deserialize)]
struct MatchQuery {
    user_id: Option<uuid::Uuid>,
}

async fn match_pool(
    State(s): State<AppState>,
    q: Result<Query<MatchQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    let q = match q {
        Ok(Query(q)) => q,
        Err(e) => {
            return problem(StatusCode::BAD_REQUEST, format!("invalid query: {e}")).into_response()
        }
    };
    let Some(uid) = q.user_id else {
        return problem(StatusCode::BAD_REQUEST, "user_id is required").into_response();
    };
    let user = match db::load_user(pool, uid).await {
        Ok(Some(u)) => u,
        Ok(None) => return problem(StatusCode::NOT_FOUND, "unknown user").into_response(),
        Err(e) => return db_error(e),
    };
    match db::match_candidates(pool, uid).await {
        Ok(pool_items) => {
            Json(serde_json::json!({ "matches": RuleMatcher.match_items(&user, &pool_items) }))
                .into_response()
        }
        Err(e) => db_error(e),
    }
}

async fn create_project(
    State(s): State<AppState>,
    raw: Result<Json<RawDrop>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    // Malformed JSON gets the same RFC 9457 shape as validation errors.
    let raw = match raw {
        Ok(Json(r)) => r,
        Err(e) => {
            return problem_errors(e.status(), "invalid JSON", vec![e.body_text()]).into_response();
        }
    };
    let valid = match validate_drop(&raw) {
        Ok(v) => v,
        Err(errs) => {
            return problem_errors(StatusCode::BAD_REQUEST, "invalid drop", errs).into_response();
        }
    };
    // A /voice/ URL must point at a file that actually exists; otherwise the
    // pool fills with dead voice links. Remote URLs are the giver's claim.
    if valid.voice_url.starts_with("/voice/") && !voice_file_exists(&s.voice_dir, &valid.voice_url)
    {
        return problem(
            StatusCode::BAD_REQUEST,
            "voice file not found under /voice/",
        )
        .into_response();
    }
    match db::create_drop(pool, &valid).await {
        Ok((project_id, offer_id)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "project_id": project_id, "offer_id": offer_id })),
        )
            .into_response(),
        Err(e) => db_error(e),
    }
}

async fn upsert_profile(
    State(s): State<AppState>,
    raw: Result<Json<RawProfile>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    let raw = match raw {
        Ok(Json(r)) => r,
        Err(e) => {
            return problem_errors(e.status(), "invalid JSON", vec![e.body_text()]).into_response();
        }
    };
    let valid = match validate_profile(&raw) {
        Ok(v) => v,
        Err(errs) => {
            return problem_errors(StatusCode::BAD_REQUEST, "invalid profile", errs)
                .into_response();
        }
    };
    match db::upsert_profile(pool, &valid).await {
        Ok(user_id) => Json(serde_json::json!({ "user_id": user_id })).into_response(),
        Err(e) => db_error(e),
    }
}

/// Readiness (for orchestrator checks): 503 unless the database answers.
/// Liveness stays on `/health`, which is always 200 with a `db` field.
async fn ready(State(s): State<AppState>) -> Response {
    match &s.pool {
        Some(p) => match sqlx::query("SELECT 1 AS one").fetch_one(p).await {
            Ok(_) => Json(serde_json::json!({ "ready": true })).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "readiness check failed");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({ "ready": false })),
                )
                    .into_response()
            }
        },
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "ready": false })),
        )
            .into_response(),
    }
}

/// Client IP for rate limiting. Behind Fly's proxy ConnectInfo is the edge
/// IP shared by everyone, so the leftmost X-Forwarded-For entry wins.
/// Trust assumption: a single honest proxy (Fly) sets the header; a client
/// spoofing it only spends from a bucket nobody else uses.
fn client_ip(req: &Request<axum::body::Body>, conn: Option<ConnectInfo<SocketAddr>>) -> IpAddr {
    if let Some(v) = req.headers().get("x-forwarded-for") {
        if let Ok(s) = v.to_str() {
            if let Some(first) = s.split(',').next() {
                if let Ok(ip) = first.trim().parse() {
                    return ip;
                }
            }
        }
    }
    conn.map(|c| c.0.ip())
        .unwrap_or_else(|| "127.0.0.1".parse().unwrap())
}

#[derive(Deserialize)]
struct SwapRequest {
    taker_id: uuid::Uuid,
    give_offer_id: uuid::Uuid,
    take_offer_id: uuid::Uuid,
}

async fn create_swap(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    raw: Result<Json<SwapRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    let raw = match raw {
        Ok(Json(r)) => r,
        Err(e) => {
            return problem_errors(e.status(), "invalid JSON", vec![e.body_text()]).into_response();
        }
    };
    let key: Option<String> = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if let Some(k) = &key {
        if k.len() > 64 {
            return problem(
                StatusCode::BAD_REQUEST,
                "idempotency-key must be 1-64 chars",
            )
            .into_response();
        }
    }
    match db::create_swap(
        pool,
        raw.taker_id,
        raw.give_offer_id,
        raw.take_offer_id,
        key.as_deref(),
    )
    .await
    {
        Ok((status, body)) => {
            let mut res = (StatusCode::from_u16(status).unwrap(), Json(body)).into_response();
            if status == 200 {
                res.headers_mut().insert(
                    axum::http::HeaderName::from_static("idempotent-replayed"),
                    HeaderValue::from_static("true"),
                );
            }
            res
        }
        Err(e) => db_error(e),
    }
}

#[derive(Deserialize)]
struct ReviveQuery {
    user_id: Option<uuid::Uuid>,
}

async fn revive(
    State(s): State<AppState>,
    q: Result<Query<ReviveQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    let q = match q {
        Ok(Query(q)) => q,
        Err(e) => {
            return problem(StatusCode::BAD_REQUEST, format!("invalid query: {e}")).into_response()
        }
    };
    let Some(uid) = q.user_id else {
        return problem(StatusCode::BAD_REQUEST, "user_id is required").into_response();
    };
    match db::latest_revive(pool, uid).await {
        Ok(Some(pkg)) => Json(pkg).into_response(),
        Ok(None) => problem(StatusCode::NOT_FOUND, "no completed swaps yet").into_response(),
        Err(e) => db_error(e),
    }
}

async fn create_report(
    State(s): State<AppState>,
    raw: Result<Json<RawReport>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(pool) = &s.pool else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response();
    };
    let raw = match raw {
        Ok(Json(r)) => r,
        Err(e) => {
            return problem_errors(e.status(), "invalid JSON", vec![e.body_text()]).into_response();
        }
    };
    let valid = match validate_report(&raw) {
        Ok(v) => v,
        Err(errs) => {
            return problem_errors(StatusCode::BAD_REQUEST, "invalid report", errs).into_response();
        }
    };
    match db::create_report(pool, valid.reporter_id, valid.project_id, &valid.reason).await {
        Ok((report_id, hidden)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "report_id": report_id, "hidden": hidden })),
        )
            .into_response(),
        Err(e) => db_error(e),
    }
}

/// 10 writes/min per IP on the write endpoints (spam-flood guard until auth).
async fn posts_limit(
    State(lim): State<Arc<RateLimiter>>,
    conn: Option<ConnectInfo<SocketAddr>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = client_ip(&req, conn);
    if let Some(retry) = lim.check(ip) {
        let (status, body) = problem(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit: 10 drops/min per IP",
        );
        let mut res = (status, body).into_response();
        res.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(retry));
        return res;
    }
    next.run(req).await
}

fn security_headers(router: Router<AppState>) -> Router<AppState> {
    router
        .layer(TimeoutLayer::new(Duration::from_secs(10)))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
}

pub fn build_app(pool: Option<sqlx::PgPool>) -> Router {
    // Separate budgets: spamming one endpoint must not eat the others' quota.
    let drop_limiter = Arc::new(RateLimiter::new(10, Duration::from_secs(60)));
    let profile_limiter = Arc::new(RateLimiter::new(10, Duration::from_secs(60)));
    let swap_limiter = Arc::new(RateLimiter::new(10, Duration::from_secs(60)));
    let report_limiter = Arc::new(RateLimiter::new(10, Duration::from_secs(60)));
    let voice_dir = std::path::PathBuf::from(
        std::env::var("VOICE_DIR").unwrap_or_else(|_| "./data/voice".into()),
    );
    let state = AppState {
        pool,
        voice_dir: voice_dir.clone(),
    };
    let router = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/projects", get(list_projects))
        .route(
            "/v1/projects",
            post(create_project)
                .route_layer(middleware::from_fn_with_state(drop_limiter, posts_limit)),
        )
        .route(
            "/v1/users/upsert",
            post(upsert_profile)
                .route_layer(middleware::from_fn_with_state(profile_limiter, posts_limit)),
        )
        .route("/v1/match", get(match_pool))
        .route("/v1/revive", get(revive))
        .route(
            "/v1/reports",
            post(create_report)
                .route_layer(middleware::from_fn_with_state(report_limiter, posts_limit)),
        )
        .route(
            "/v1/swaps",
            post(create_swap)
                .route_layer(middleware::from_fn_with_state(swap_limiter, posts_limit)),
        )
        .nest_service("/voice", ServeDir::new(&voice_dir))
        .fallback(fallback_404);
    security_headers(router).with_state(state)
}

/// Unmatched paths: trim one trailing slash via 308 (idempotent, so no
/// loops), otherwise return a shaped JSON 404 instead of Axum's default text.
async fn fallback_404(uri: axum::http::Uri) -> Response {
    let path = uri.path();
    if path.len() > 1 && path.ends_with('/') {
        let q = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
        let to = format!("{}{}", path.trim_end_matches('/'), q);
        if let Ok(loc) = HeaderValue::from_str(&to) {
            let mut res = (StatusCode::PERMANENT_REDIRECT, "").into_response();
            res.headers_mut().insert(header::LOCATION, loc);
            return res;
        }
    }
    problem(StatusCode::NOT_FOUND, "not found").into_response()
}

/// Defense in depth next to validation: refuse /voice/ URLs whose file is
/// absent (or escapes the dir) so the pool never lists dead voice links.
fn voice_file_exists(dir: &std::path::Path, url: &str) -> bool {
    let rel = url.strip_prefix("/voice/").unwrap_or("");
    if rel.is_empty() || rel.contains("..") {
        return false;
    }
    dir.join(rel).is_file()
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        // Empty EnvFilter silences everything; default to warn so degraded
        // mode (DB down) is always visible in logs.
        std::env::set_var("RUST_LOG", "questdrop=warn,tower_http=warn");
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let pool = match std::env::var("DATABASE_URL") {
        Ok(url) => match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(3))
            .idle_timeout(Some(Duration::from_secs(60)))
            .after_connect(|conn, _| {
                Box::pin(async move {
                    sqlx::query("SET statement_timeout = '5s'")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
        {
            Ok(p) => {
                sqlx::migrate!("./migrations").run(&p).await?;
                Some(p)
            }
            Err(e) => {
                warn!("DB unreachable, running without pool: {e}");
                None
            }
        },
        Err(_) => {
            warn!("DATABASE_URL unset, running without pool");
            None
        }
    };

    let voice_dir = std::env::var("VOICE_DIR").unwrap_or_else(|_| "./data/voice".into());
    std::fs::create_dir_all(&voice_dir)?;

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e} (is another server on this port?)"))?;
    tracing::info!("listening on {addr}");
    axum::serve(
        listener,
        build_app(pool).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = term.recv() => {},
                _ = tokio::signal::ctrl_c() => {},
            },
            // Signal handling unavailable: still shut down cleanly on Ctrl-C.
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
