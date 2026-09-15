//! Phase 1 integration tests: Drop -> Freeze -> Pool.
//! Needs a Postgres database:
//!   TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/questdrop_test" cargo test
//! Tests share one database and run serially (DB_LOCK) with truncate between them.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use questdrop::build_app;
use sqlx::PgPool;
use std::sync::OnceLock;
use tower::ServiceExt;

static DB_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
    DB_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
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

async fn post_drop(app: &axum::Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
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
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    let (status, json) =
        post_drop(&app, drop_json("alice_1", "alice1@x.com", "Photo renamer")).await;
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
        .oneshot(
            Request::builder()
                .uri("/v1/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let pool_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(pool_json["pool"].as_array().unwrap().len(), 1);
    assert_eq!(pool_json["pool"][0]["giver_handle"], "alice_1");
}

#[tokio::test]
async fn invalid_drop_400_with_errors() {
    let _g = lock().await;
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
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    // Fresh app per request would reset the IP limiter; one app for all 4 posts.
    let app = build_app(Some(pool));
    for i in 1..=3 {
        let (s, _) = post_drop(
            &app,
            drop_json("bob_4", "bob4@x.com", &format!("Project {i}")),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, _) = post_drop(&app, drop_json("bob_4", "bob4@x.com", "Project 4")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn email_handle_mismatch_409() {
    let _g = lock().await;
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
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool));
    for i in 1..=3 {
        let (s, _) = post_drop(
            &app,
            drop_json("dave_6", "dave6@x.com", &format!("Item {i}")),
        )
        .await;
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
    let cursor = p1["next_cursor"]
        .as_str()
        .expect("must have cursor")
        .to_string();
    let p2 = get(format!("/v1/projects?limit=2&cursor={cursor}")).await;
    assert_eq!(p2["pool"].as_array().unwrap().len(), 1);
    assert!(p2.get("next_cursor").unwrap().is_null());
}

#[tokio::test]
async fn malformed_json_is_rfc9457() {
    let _g = lock().await;
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
    let _g = lock().await;
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
async fn profile_upsert_drives_match_fit() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    // Skills profile first: B can do rust, A posts a rust quest.
    let ck_b = login(&app, &pool, "fitb@x.com", Some("fitter_b")).await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/users/upsert")
                .header("content-type", "application/json")
                .header("cookie", format!("qd_session={ck_b}"))
                .body(Body::from(
                    serde_json::json!({
                        "handle": "fitter_b", "email": "fitb@x.com",
                        "can_do": ["rust"], "looking_for": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 65536).await.unwrap();
    let uid: String = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["user_id"]
        .as_str()
        .unwrap()
        .to_string();

    let (s, _) = post_drop(&app, drop_json("fitter_a", "fita@x.com", "Rust quest")).await;
    assert_eq!(s, StatusCode::CREATED);

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
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let items = json["matches"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    // fit = (overlap(can_do=[rust],[rust]) + overlap([],[rust])) / 2 = 0.50
    assert!(items[0]["reason"].as_str().unwrap().starts_with("fit=0.50"));

    // Same email + different handle with B's session -> 403, not a takeover.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/users/upsert")
                .header("content-type", "application/json")
                .header("cookie", format!("qd_session={ck_b}"))
                .body(Body::from(
                    serde_json::json!({
                        "handle": "impostor", "email": "fitb@x.com",
                        "can_do": [], "looking_for": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // No session at all -> 401.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/users/upsert")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "handle": "fitter_b", "email": "fitb@x.com",
                        "can_do": [], "looking_for": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn concurrent_drops_hold_cap_without_5xx() {
    // 10 parallel first-drops, one identity: exactly 3 win (201), the rest
    // get 429 — and no 503 from aborted-transaction retries.
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    let mut tasks = Vec::new();
    for i in 0..10 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            post_drop(
                &app,
                drop_json("racer", "racer@x.com", &format!("Race {i}")),
            )
            .await
            .0
        }));
    }
    let mut created = 0;
    let mut capped = 0;
    for t in tasks {
        match t.await.unwrap() {
            StatusCode::CREATED => created += 1,
            StatusCode::TOO_MANY_REQUESTS => capped += 1,
            s => panic!("unexpected status under concurrency: {s}"),
        }
    }
    assert_eq!(created, 3, "daily cap must hold exactly");
    assert_eq!(capped, 7);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3);
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email='racer@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1, "one identity despite the insert race");
}

async fn post_swap(
    app: &axum::Router,
    body: serde_json::Value,
    key: Option<&str>,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, bool) {
    let mut b = Request::builder()
        .method("POST")
        .uri("/v1/swaps")
        .header("content-type", "application/json");
    if let Some(k) = key {
        b = b.header("idempotency-key", k);
    }
    if let Some(c) = cookie {
        b = b.header("cookie", format!("qd_session={c}"));
    }
    let res = app
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let replayed = res.headers().get("idempotent-replayed").is_some();
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, json, replayed)
}

/// Full login loop: db-issued token (LogMailer in tests) redeemed over HTTP.
/// Returns the raw session cookie value.
async fn login(app: &axum::Router, pool: &PgPool, email: &str, handle: Option<&str>) -> String {
    let raw = questdrop::auth::issue_magic_token(pool, email, handle)
        .await
        .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/callback")
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"token":"{raw}"}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let set = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(set.starts_with("qd_session="), "HttpOnly session cookie");
    assert!(set.contains("HttpOnly"));
    set.split(';').next().unwrap()["qd_session=".len()..].to_string()
}

fn swap_json(taker: &str, give: &str, take: &str) -> serde_json::Value {
    serde_json::json!({"taker_id": taker, "give_offer_id": give, "take_offer_id": take})
}

/// Two users with one OPEN offer each. Returns (uid_b, offer_b, offer_a).
async fn seed_swap_pair(app: &axum::Router, pool: &PgPool, tag: &str) -> (String, String, String) {
    let (s, _) = post_drop(
        app,
        drop_json(
            &format!("sw_a_{tag}"),
            &format!("swa{tag}@x.com"),
            "A quest",
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = post_drop(
        app,
        drop_json(
            &format!("sw_b_{tag}"),
            &format!("swb{tag}@x.com"),
            "B quest",
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let uid_b: String = sqlx::query_scalar(&format!(
        "SELECT id::text FROM users WHERE handle='sw_b_{tag}'"
    ))
    .fetch_one(pool)
    .await
    .unwrap();
    let offer_b: String = sqlx::query_scalar(&format!(
        "SELECT o.id::text FROM swap_offers o JOIN users u ON u.id=o.giver_id WHERE u.handle='sw_b_{tag}'"
    ))
    .fetch_one(pool)
    .await
    .unwrap();
    let offer_a: String = sqlx::query_scalar(&format!(
        "SELECT o.id::text FROM swap_offers o JOIN users u ON u.id=o.giver_id WHERE u.handle='sw_a_{tag}'"
    ))
    .fetch_one(pool)
    .await
    .unwrap();
    (uid_b, offer_b, offer_a)
}

#[tokio::test]
async fn swap_201_freezes_both_sides() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "s1").await;
    let ck_b = login(&app, &pool, "swbs1@x.com", None).await;

    let (s, json, replayed) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "body: {json}");
    assert!(!replayed);
    assert!(json.get("match_id").is_some());

    let (offers, projects, matches, events): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM swap_offers WHERE status='MATCHED'::offer_status),
                (SELECT COUNT(*) FROM projects WHERE status='MATCHED'::project_status),
                (SELECT COUNT(*) FROM matches),
                (SELECT COUNT(*) FROM outbox_events WHERE type='swap.completed')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((offers, projects, matches, events), (2, 2, 1, 1));
}

#[tokio::test]
async fn swap_double_claim_409() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "s2").await;
    let ck_b = login(&app, &pool, "swbs2@x.com", None).await;

    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    // Same take again with a fresh giver -> taken already.
    let (s, _) = post_drop(&app, drop_json("sw_c_s2", "swcs2@x.com", "C quest")).await;
    assert_eq!(s, StatusCode::CREATED);
    let uid_c: String = sqlx::query_scalar("SELECT id::text FROM users WHERE handle='sw_c_s2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let offer_c: String = sqlx::query_scalar(
        "SELECT o.id::text FROM swap_offers o JOIN users u ON u.id=o.giver_id WHERE u.handle='sw_c_s2'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let ck_c = login(&app, &pool, "swcs2@x.com", None).await;
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_c, &offer_c, &offer_a),
        None,
        Some(&ck_c),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Replaying your own matched give -> also 409.
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn swap_replay_returns_stored_200() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "s3").await;
    let ck_b = login(&app, &pool, "swbs3@x.com", None).await;

    let (s1, j1, r1) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        Some("key-s3-abc"),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);
    assert!(!r1);
    let (s2, j2, r2) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        Some("key-s3-abc"),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);
    assert!(r2);
    assert_eq!(j1["match_id"], j2["match_id"]);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "replay must not execute twice");
}

#[tokio::test]
async fn swap_rules_4xx() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "s4").await;
    let ck_b = login(&app, &pool, "swbs4@x.com", None).await;

    // give == take
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_b),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    // give is not yours
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_a, &offer_b),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    // session user wins over body ids: unknown taker in body -> 403, not 404.
    let (s, _, _) = post_swap(
        &app,
        swap_json("00000000-0000-0000-0000-000000000000", &offer_b, &offer_a),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // unknown offer still 404 (actor matches).
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, "00000000-0000-0000-0000-000000000000"),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // oversized key
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        Some(&"k".repeat(65)),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // no session at all -> 401.
    let (s, _, _) = post_swap(&app, swap_json(&uid_b, &offer_b, &offer_a), None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn swap_race_single_winner() {
    // 5 takers race for one offer: exactly one 201, rest 409, zero 5xx.
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (_, _, offer_a) = seed_swap_pair(&app, &pool, "s5").await;
    let mut giver_offers = Vec::new();
    for i in 0..5 {
        let h = format!("racer_s5_{i}");
        let em = format!("rs5{i}@x.com");
        let (s, _) = post_drop(&app, drop_json(&h, &em, "Racer")).await;
        assert_eq!(s, StatusCode::CREATED);
        let ck = login(&app, &pool, &em, None).await;
        let uid: String =
            sqlx::query_scalar(&format!("SELECT id::text FROM users WHERE handle='{h}'"))
                .fetch_one(&pool)
                .await
                .unwrap();
        let offer: String = sqlx::query_scalar(&format!(
            "SELECT o.id::text FROM swap_offers o JOIN users u ON u.id=o.giver_id WHERE u.handle='{h}'"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        giver_offers.push((uid, offer, ck));
    }
    let mut tasks = Vec::new();
    for (uid, offer, ck) in giver_offers {
        let app = app.clone();
        let take = offer_a.clone();
        tasks.push(tokio::spawn(async move {
            post_swap(&app, swap_json(&uid, &offer, &take), None, Some(&ck))
                .await
                .0
        }));
    }
    let mut won = 0;
    let mut lost = 0;
    for t in tasks {
        match t.await.unwrap() {
            StatusCode::CREATED => won += 1,
            StatusCode::CONFLICT => lost += 1,
            s => panic!("unexpected status in swap race: {s}"),
        }
    }
    assert_eq!(won, 1);
    assert_eq!(lost, 4);
}

async fn post_report(
    app: &axum::Router,
    body: serde_json::Value,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder()
        .method("POST")
        .uri("/v1/reports")
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        b = b.header("cookie", format!("qd_session={c}"));
    }
    let res = app
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, json)
}

async fn upsert(app: &axum::Router, handle: &str, email: &str, cookie: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/users/upsert")
                .header("content-type", "application/json")
                .header("cookie", format!("qd_session={cookie}"))
                .body(
                    Body::from(
                        serde_json::json!({"handle": handle, "email": email, "can_do": [], "looking_for": []})
                            .to_string(),
                    ),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 65536).await.unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["user_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn third_report_hides_project() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    let (s, _) = post_drop(&app, drop_json("rep_a", "repa@x.com", "Flagged quest")).await;
    assert_eq!(s, StatusCode::CREATED);
    let project: String = sqlx::query_scalar(
        "SELECT p.id::text FROM projects p JOIN users u ON u.id=p.giver_id WHERE u.handle='rep_a'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mk = |uid: &str| serde_json::json!({"reporter_id": uid, "project_id": project, "reason": "This looks like spam content here"});

    let ck_b = login(&app, &pool, "repb@x.com", Some("rep_b")).await;
    let uid_b = upsert(&app, "rep_b", "repb@x.com", &ck_b).await;
    let (s, j) = post_report(&app, mk(&uid_b), Some(&ck_b)).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(j["hidden"], false);
    // Duplicate + own-report + unknown ids + short reason.
    let (s, _) = post_report(&app, mk(&uid_b), Some(&ck_b)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let uid_a: String = sqlx::query_scalar("SELECT id::text FROM users WHERE handle='rep_a'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ck_a = login(&app, &pool, "repa@x.com", None).await;
    let (s, _) = post_report(&app, mk(&uid_a), Some(&ck_a)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = post_report(
        &app,
        serde_json::json!({"reporter_id": "00000000-0000-0000-0000-000000000000", "project_id": project, "reason": "This looks like spam content here"}),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, j) = post_report(
        &app,
        serde_json::json!({"reporter_id": uid_b, "project_id": project, "reason": "short"}),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(j["errors"].is_array());

    let ck_c = login(&app, &pool, "repc@x.com", Some("rep_c")).await;
    let uid_c = upsert(&app, "rep_c", "repc@x.com", &ck_c).await;
    let ck_d = login(&app, &pool, "repd@x.com", Some("rep_d")).await;
    let uid_d = upsert(&app, "rep_d", "repd@x.com", &ck_d).await;
    let (s, _) = post_report(&app, mk(&uid_c), Some(&ck_c)).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, j) = post_report(&app, mk(&uid_d), Some(&ck_d)).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(j["hidden"], true);

    // Hidden from pool and unswappable.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let pool_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(pool_json["pool"].as_array().unwrap().len(), 0);
    let offer: String = sqlx::query_scalar(
        "SELECT o.id::text FROM swap_offers o JOIN projects p ON p.id=o.project_id JOIN users u ON u.id=p.giver_id WHERE u.handle='rep_a'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, _, _) = post_swap(&app, swap_json(&uid_c, &offer, &offer), None, Some(&ck_c)).await;
    assert_ne!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn revive_returns_package_and_task() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "rv").await;
    let ck_b = login(&app, &pool, "swbrv@x.com", None).await;
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    // Same taker takes again: history keeps both revives, newest first.
    let (s, _) = post_drop(&app, drop_json("sw_c_rv", "swcrv@x.com", "Second")).await;
    assert_eq!(s, StatusCode::CREATED);
    let offer_c: String = sqlx::query_scalar(
        "SELECT o.id::text FROM swap_offers o JOIN users u ON u.id=o.giver_id WHERE u.handle='sw_c_rv'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, _) = post_drop(&app, drop_json("sw_b_rv", "swbrv@x.com", "B quest 2")).await;
    assert_eq!(s, StatusCode::CREATED);
    let offer_b2: String = sqlx::query_scalar(
        "SELECT o.id::text FROM swap_offers o JOIN projects p ON p.id=o.project_id WHERE p.title='B quest 2'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b2, &offer_c),
        None,
        Some(&ck_b),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/revive?user_id={uid_b}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let revives = json["revives"].as_array().unwrap();
    assert_eq!(revives.len(), 2);
    assert_eq!(revives[0]["project"]["title"], "Second");
    assert_eq!(revives[1]["project"]["title"], "A quest");
    assert_eq!(revives[0]["first_task"]["minutes"], 2);
    assert_eq!(
        revives[0]["first_task"]["steps"].as_array().unwrap().len(),
        4
    );

    // Giver with no takes + unknown user -> 404.
    let uid_a: String = sqlx::query_scalar("SELECT id::text FROM users WHERE handle='sw_a_rv'")
        .fetch_one(&pool)
        .await
        .unwrap();
    for uri in [
        format!("/v1/revive?user_id={uid_a}"),
        "/v1/revive?user_id=00000000-0000-0000-0000-000000000000".to_string(),
        "/v1/revive".to_string(),
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
async fn swap_key_reuse_with_different_params_400() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "s6").await;
    let ck_b = login(&app, &pool, "swbs6@x.com", None).await;
    let (s, _) = post_drop(&app, drop_json("sw_c_s6", "swcs6@x.com", "C quest")).await;
    assert_eq!(s, StatusCode::CREATED);
    let offer_c: String = sqlx::query_scalar(
        "SELECT o.id::text FROM swap_offers o JOIN users u ON u.id=o.giver_id WHERE u.handle='sw_c_s6'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let (s1, _, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_a),
        Some("key-s6-dup"),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);
    // Same key, different take -> must fail, never replay another result.
    let (s2, j2, _) = post_swap(
        &app,
        swap_json(&uid_b, &offer_b, &offer_c),
        Some("key-s6-dup"),
        Some(&ck_b),
    )
    .await;
    assert_eq!(s2, StatusCode::BAD_REQUEST);
    assert!(j2["detail"]
        .as_str()
        .unwrap()
        .contains("different parameters"));
}

#[tokio::test]
async fn method_not_allowed_is_shaped_405() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool));
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/match")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    let bytes = to_bytes(res.into_body(), 65536).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], 405);
    assert_eq!(json["title"], "Method Not Allowed");
}

#[tokio::test]
async fn same_key_race_executes_once() {
    // 5 parallel identical keyed swaps: exactly one 201, four 200 replays,
    // a single match row. The PK gate serializes the race.
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (uid_b, offer_b, offer_a) = seed_swap_pair(&app, &pool, "s8").await;
    let ck_b = login(&app, &pool, "swbs8@x.com", None).await;
    let body = swap_json(&uid_b, &offer_b, &offer_a);

    let mut tasks = Vec::new();
    for _ in 0..5 {
        let app = app.clone();
        let body = body.clone();
        let ck = ck_b.clone();
        tasks.push(tokio::spawn(async move {
            post_swap(&app, body, Some("key-s8-race"), Some(&ck)).await
        }));
    }
    let mut fresh = 0;
    let mut replayed = 0;
    let mut ids = std::collections::HashSet::new();
    for t in tasks {
        let (s, j, r) = t.await.unwrap();
        ids.insert(j["match_id"].clone());
        match (s, r) {
            (StatusCode::CREATED, false) => fresh += 1,
            (StatusCode::OK, true) => replayed += 1,
            (s, r) => panic!("unexpected race outcome: {s} replayed={r}"),
        }
    }
    assert_eq!(fresh, 1);
    assert_eq!(replayed, 4);
    assert_eq!(ids.len(), 1, "all responses carry the same match");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn publisher_delivers_then_idles() {
    use questdrop::notify::{OutboxEvent, Sink};
    struct VecSink {
        out: std::sync::Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl Sink for VecSink {
        async fn send(&self, event: &OutboxEvent) -> Result<(), String> {
            self.out.lock().unwrap().push(event.kind.clone());
            Ok(())
        }
    }

    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (s, _) = post_drop(&app, drop_json("pub_a", "puba@x.com", "Pub quest")).await;
    assert_eq!(s, StatusCode::CREATED);

    let sink = VecSink {
        out: std::sync::Mutex::new(Vec::new()),
    };
    let n = questdrop::publisher::run_once(&pool, &sink).await.unwrap();
    assert!(n >= 1, "must deliver the swap.created event");
    assert!(sink
        .out
        .lock()
        .unwrap()
        .contains(&"swap.created".to_string()));
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox_events WHERE NOT processed")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    let n2 = questdrop::publisher::run_once(&pool, &sink).await.unwrap();
    assert_eq!(n2, 0, "second poll idles");
}

#[tokio::test]
async fn publisher_keeps_failed_events() {
    use questdrop::notify::{OutboxEvent, Sink};
    struct FailSink;
    #[async_trait::async_trait]
    impl Sink for FailSink {
        async fn send(&self, _event: &OutboxEvent) -> Result<(), String> {
            Err("downstream down".into())
        }
    }

    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (s, _) = post_drop(&app, drop_json("pub_b", "pubb@x.com", "Pub quest")).await;
    assert_eq!(s, StatusCode::CREATED);

    let n = questdrop::publisher::run_once(&pool, &FailSink)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox_events WHERE NOT processed")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(left >= 1, "failed events stay queued, nothing lost");
}

#[tokio::test]
async fn match_embedding_algo_is_deterministic() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));
    let (s, _) = post_drop(&app, drop_json("emb_a", "emba@x.com", "Rust quest")).await;
    assert_eq!(s, StatusCode::CREATED);
    let uid: String = sqlx::query_scalar("SELECT id::text FROM users WHERE handle='emb_a'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let get = |uri: String| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            (res.status(), {
                let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
            })
        }
    };
    // Own project excluded -> empty; bad algo -> 400.
    let (s, _) = get(format!("/v1/match?user_id={uid}&algo=embedding")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = get(format!("/v1/match?user_id={uid}&algo=nope")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn auth_request_callback_me_loop() {
    let _g = lock().await;
    let pool = test_pool().await;
    clean(&pool).await;
    let app = build_app(Some(pool.clone()));

    // Request accepts new identity, rejects garbage.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/request")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"newbie@x.com","handle":"newbie"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/request")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"nope"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Callback redeems once; replay and garbage fail.
    let raw = questdrop::auth::issue_magic_token(&pool, "newbie@x.com", Some("newbie"))
        .await
        .unwrap();
    let cb_app = app.clone();
    let cb = move |tok: String| {
        let app = cb_app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/callback")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"token":"{tok}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let res = cb(raw.clone()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("qd_session="));
    assert_eq!(cb(raw).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        cb("garbage-token".into()).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // /v1/me with and without the session.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let session = cookie.split(';').next().unwrap().to_string();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("cookie", session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 65536).await.unwrap();
    let me: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(me["handle"], "newbie");
    assert_eq!(me["email"], "newbie@x.com");

    // Expired token is dead.
    let raw2 = questdrop::auth::issue_magic_token(&pool, "old@x.com", None)
        .await
        .unwrap();
    sqlx::query("UPDATE magic_tokens SET expires_at = now() - interval '1 min'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(cb(raw2).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn db_down_503() {
    let app = build_app(None);
    let (status, _) = post_drop(&app, drop_json("erin_7", "erin7@x.com", "Nope")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
