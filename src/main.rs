pub mod matcher;

use std::net::SocketAddr;

use askama::Template;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use tower_http::services::ServeDir;
use tracing::warn;

#[derive(Clone)]
struct AppState {
    pool: Option<sqlx::PgPool>,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    pool_count: usize,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Serialize)]
struct Problem {
    #[serde(rename = "type")]
    type_: &'static str,
    title: &'static str,
    status: u16,
    detail: String,
}

fn problem(status: StatusCode, detail: impl Into<String>) -> impl IntoResponse {
    let body = Problem {
        type_: "about:blank",
        title: status.canonical_reason().unwrap_or("Error"),
        status: status.as_u16(),
        detail: detail.into(),
    };
    (status, Json(body))
}

async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let pool_count = 0;
    let _ = state.pool.as_ref();
    match (IndexTemplate { pool_count }).render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(Health { status: "ok" })
}

async fn list_projects() -> impl IntoResponse {
    // Phase 1 wires this to Postgres. Contract stays fixed for Bot (B).
    Json(serde_json::json!({ "pool": [] }))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct MatchQuery {
    user_id: Option<String>,
}

async fn match_pool(Query(_q): Query<MatchQuery>) -> impl IntoResponse {
    // RuleMatcher lives in src/matcher.rs; empty pool until Phase 1 seeds DB.
    Json(serde_json::json!({ "matches": [] }))
}

async fn create_project() -> impl IntoResponse {
    problem(
        StatusCode::NOT_IMPLEMENTED,
        "DB not wired yet (Phase 1). Send link_url + voice file, time_bucket, energy.",
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    dotenvy::dotenv().ok();

    let pool = match std::env::var("DATABASE_URL") {
        Ok(url) => match PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
        {
            Ok(p) => Some(p),
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

    let state = AppState { pool };
    let voice_dir = std::env::var("VOICE_DIR").unwrap_or_else(|_| "./data/voice".into());
    let _ = std::fs::create_dir_all(&voice_dir);

    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/v1/projects", get(list_projects))
        .route("/v1/projects", post(create_project))
        .route("/v1/match", get(match_pool))
        .nest_service("/voice", ServeDir::new(voice_dir))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("listening on {addr}");
    axum::serve(
        tokio::net::TcpListener::bind(addr).await.unwrap(),
        app.into_make_service(),
    )
    .await
    .unwrap();
}
