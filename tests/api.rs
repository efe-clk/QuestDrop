//! Phase 1 integration tests: Drop -> Freeze -> Pool.
//! Needs a Postgres database:
//!   TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/questdrop_test" cargo test
//! Tests share one database and run serially (DB_LOCK) with truncate between them.

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use questdrop::build_app;
use sqlx::PgPool;
use std::sync::{Mutex, OnceLock};
use tower::ServiceExt;

static DB_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn lock() -> std::sync::MutexGuard<'static, ()> {
    DB_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

async fn test_pool() -> PgPool {
    // Safety: tests TRUNCATE every table. Only a throwaway DB whose name
    // contains "test" is accepted — never dev/prod.
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set (e.g. .../questdrop_test)");
    if !url.to_lowercase().contains("test") {
        panic!("refusing to run destructive tests against non-test database");
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(3))
        .connect(&url)
        .await
        .expect("test database must be reachable");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations must apply");
    pool
}

async fn clean(pool: &PgPool) {
    sqlx::query("TRUNCATE users, projects, swap_offers, matches, outbox_events, reports CASCADE")
        .execute(pool)
        .await
        .unwrap();
    // Fixture for /voice/* existence checks (gitignored runtime dir).
    std::fs::create_dir_all("./data/voice").unwrap();
    std::fs::write("./data/voice/demo.mp3", b"fake-mp3-fixture").unwrap();
}

fn drop_json(handle: &str, email: &str, title: &str) -> serde_json::Value {
    serde_json::json!({
        "handle": handle,
        "email": email,
        "title": title,
        "one_liner": "A half-finished tool that needs a new owner to continue",
        "link_url": "https://github.com/x/y",
        "voice_url": "/voice/demo.mp3",
        "skill_needed": ["rust"],
        "time_bucket": "S",
        "energy": "LOW",
    })
}

async fn post_drop(
    app: &axum::Router,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/projects")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, json)
}

#[tokio::test]
async fn drop_201_freezes_offer_and_outbox() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    let (status, json) = post_drop(&app, drop_json("alice_1", "alice1@x.com", "Photo renamer")).await;
    assert_eq!(status, StatusCode::CREATED, "body: {json}");
    assert!(json.get("project_id").is_some());
    assert!(json.get("offer_id").is_some());

    let (offers, events): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM swap_offers WHERE status='OPEN'::offer_status),
                (SELECT COUNT(*) FROM outbox_events WHERE type='swap.created' AND NOT processed)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((offers, events), (1, 1));

    // Pool lists it.
    let res = app
        .clone()
        .oneshot(Request::builder().uri("/v1/projects").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let pool_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(pool_json["pool"].as_array().unwrap().len(), 1);
    assert_eq!(pool_json["pool"][0]["giver_handle"], "alice_1");
}

#[tokio::test]
async fn invalid_drop_400_with_errors() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool));

    let (status, json) = post_drop(
        &app,
        serde_json::json!({"handle": "AB", "email": "nope", "title": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["errors"].as_array().unwrap().len() >= 4);
}

#[tokio::test]
async fn fourth_drop_per_day_429() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    // Fresh app per request would reset the IP limiter; one app for all 4 posts.
    let app = build_app(Some(pool));
    for i in 1..=3 {
        let (s, _) = post_drop(&app, drop_json("bob_4", "bob4@x.com", &format!("Project {i}"))).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, _) = post_drop(&app, drop_json("bob_4", "bob4@x.com", "Project 4")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn email_handle_mismatch_409() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool));
    let (s, _) = post_drop(&app, drop_json("carol_5", "carol5@x.com", "First")).await;
    assert_eq!(s, StatusCode::CREATED);
    // Same email, different handle -> identity conflict, no silent takeover.
    let (s, _) = post_drop(&app, drop_json("mallory_5", "carol5@x.com", "Hijack")).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn pool_paginates_with_cursor() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool));
    for i in 1..=3 {
        let (s, _) = post_drop(&app, drop_json("dave_6", "dave6@x.com", &format!("Item {i}"))).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let get = |uri: String| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        }
    };
    let p1 = get("/v1/projects?limit=2".into()).await;
    assert_eq!(p1["pool"].as_array().unwrap().len(), 2);
    let cursor = p1["next_cursor"].as_str().expect("must have cursor").to_string();
    let p2 = get(format!("/v1/projects?limit=2&cursor={cursor}")).await;
    assert_eq!(p2["pool"].as_array().unwrap().len(), 1);
    assert!(p2.get("next_cursor").unwrap().is_null());
}

#[tokio::test]
async fn malformed_json_is_rfc9457() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool));
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/projects")
                .header("content-type", "application/json")
                .body(Body::from("{bad json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(res.into_body(), 65536).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], 400);
    assert!(json.get("title").is_some());
}

#[tokio::test]
async fn match_returns_ranked_excluding_own() {
    let _g = lock();
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    let (s, _) = post_drop(&app, drop_json("matcher_a", "matcha@x.com", "Rust CLI")).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = post_drop(&app, drop_json("matcher_a", "matcha@x.com", "Rust bot")).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = post_drop(&app, drop_json("matcher_b", "matchb@x.com", "Go tool")).await;
    assert_eq!(s, StatusCode::CREATED);

    let uid: String = sqlx::query_scalar("SELECT id::text FROM users WHERE handle='matcher_b'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/match?user_id={uid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let items = json["matches"].as_array().unwrap();
    // Only A's two projects (own excluded), scores in range.
    assert_eq!(items.len(), 2);
    for m in items {
        let s = m["score"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&s));
    }

    // Unknown user -> 404, garbage uuid -> 400, missing param -> 400.
    for uri in [
        "/v1/match?user_id=00000000-0000-0000-0000-000000000000".to_string(),
        "/v1/match?user_id=nope".to_string(),
        "/v1/match".to_string(),
    ] {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(res.status().is_client_error(), "{uri}");
    }
}

#[tokio::test]
async fn db_down_503() {
    let app = build_app(None);
    let (status, _) = post_drop(&app, drop_json("erin_7", "erin7@x.com", "Nope")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
