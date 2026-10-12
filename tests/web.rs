//! Web UI integration tests (docs/web-ui.md §12): server-rendered pages,
//! form posts, redirects, CSRF and HTMX behaviour against a real Postgres.
//!   TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/questdrop_test" cargo test
//! Browser-level E2E (recorder, two users, race in real browsers) lives in
//! `e2e/` and runs against a live server.

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
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
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set (e.g. .../questdrop_test)");
    if !url.to_lowercase().contains("test") {
        panic!("refusing to run destructive tests against non-test database");
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("test database must be reachable");
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query(
        "TRUNCATE users, projects, swap_offers, matches, outbox_events, reports, magic_tokens, sessions CASCADE",
    )
    .execute(&pool)
    .await
    .unwrap();
    std::fs::create_dir_all("./data/voice").unwrap();
    std::fs::write(
        "./data/voice/demo.mp3",
        b"ID3\x04\x00\x00\x00\x00\x00\x00fixture",
    )
    .unwrap();
    pool
}

struct Res {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Res {
    fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .or_else(|| self.headers.get("hx-location"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

async fn send(app: &Router, req: Request<Body>) -> Res {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), 4 * 1024 * 1024).await.unwrap();
    Res {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri).header("accept", "text/html");
    if let Some(c) = cookie {
        b = b.header("cookie", format!("qd_session={c}"));
    }
    b.body(Body::empty()).unwrap()
}

fn post_form(uri: &str, cookie: Option<&str>, form: &[(&str, &str)]) -> Request<Body> {
    let body: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("sec-fetch-site", "same-origin");
    if let Some(c) = cookie {
        b = b.header("cookie", format!("qd_session={c}"));
    }
    b.body(Body::from(body)).unwrap()
}

const BOUNDARY: &str = "qdboundary7MA4YWxkTrZu0gW";

fn post_multipart(
    uri: &str,
    cookie: Option<&str>,
    fields: &[(&str, &str)],
    file: Option<&[u8]>,
) -> Request<Body> {
    let mut body: Vec<u8> = Vec::new();
    for (k, v) in fields {
        body.extend_from_slice(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    if let Some(bytes) = file {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"voice_file\"; filename=\"n.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("sec-fetch-site", "same-origin");
    if let Some(c) = cookie {
        b = b.header("cookie", format!("qd_session={c}"));
    }
    b.body(Body::from(body)).unwrap()
}

/// Logs in through the web form flow: POST /login, then the confirm POST.
async fn web_login(app: &Router, pool: &PgPool, email: &str, handle: &str) -> String {
    // Token comes from the DB layer (the LogMailer only logs the link).
    let token = questdrop::auth::issue_magic_token(pool, email, Some(handle))
        .await
        .unwrap();
    let r = send(
        app,
        post_form(
            "/login/confirm",
            None,
            &[("token", &token), ("next", "/drop")],
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    assert_eq!(r.location(), "/drop");
    let set = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    set.split(';').next().unwrap()["qd_session=".len()..].to_string()
}

fn drop_fields(title: &str) -> Vec<(&str, &str)> {
    vec![
        ("title", title),
        ("one_liner", "A half-finished tool that needs a new owner"),
        ("link_url", "https://github.com/x/y"),
        ("voice_url", "/voice/demo.mp3"),
        ("skills", "Rust, cli"),
        ("time_bucket", "S"),
        ("energy", "LOW"),
        ("next", ""),
    ]
}

async fn web_drop(app: &Router, ck: &str, title: &str) -> String {
    let mut req = post_multipart("/drop", Some(ck), &drop_fields(title), None);
    // Separate IP buckets so bulk seeding doesn't hit the 10/min limiter.
    let ip = format!(
        "10.0.{}.{}",
        title.len(),
        title.bytes().map(u32::from).sum::<u32>() % 250
    );
    req.headers_mut()
        .insert("x-forwarded-for", ip.parse().unwrap());
    let r = send(app, req).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let loc = r.location().to_string();
    assert!(
        loc.starts_with("/q/") && loc.ends_with("?dropped=1"),
        "{loc}"
    );
    loc
}

async fn offer_of(pool: &PgPool, title: &str) -> String {
    sqlx::query_scalar(
        "SELECT o.id::text FROM swap_offers o JOIN projects p ON p.id = o.project_id WHERE p.title = $1",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn pages_render_and_gate_on_login() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));

    let r = send(&app, get("/", None)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<html lang=\"en\""));
    assert!(r.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let csp = r.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"));
    assert!(r.body.contains("/static/vendor/htmx-2.0.10.min.js"));
    assert!(
        !r.body.contains("/v1/match"),
        "web-ui §10.9: no dead API link"
    );
    assert!(!r.body.contains("<script>"), "no inline scripts under CSP");

    // Turkish via Accept-Language, then the cookie wins.
    let tr = Request::builder()
        .uri("/")
        .header("accept-language", "tr-TR,tr;q=0.9")
        .body(Body::empty())
        .unwrap();
    let r = send(&app, tr).await;
    assert!(r.body.contains("<html lang=\"tr\"") && r.body.contains("Quest bırak"));
    let r = send(&app, get("/lang/tr?next=/match", None)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), "/match");
    assert!(r.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("qd_lang=tr"));
    let r = send(&app, get("/lang/en?next=//evil.example", None)).await;
    assert_eq!(r.location(), "/", "open redirect blocked");

    // Login-gated pages redirect with a safe next.
    for (path, next) in [
        ("/drop", "%2Fdrop"),
        ("/match", "%2Fmatch"),
        ("/revive", "%2Frevive"),
        ("/me", "%2Fme"),
    ] {
        let r = send(&app, get(path, None)).await;
        assert_eq!(r.status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(r.location(), format!("/login?next={next}"), "{path}");
    }
    // HTMX gets HX-Location instead of a 303.
    let hx = Request::builder()
        .uri("/drop")
        .header("hx-request", "true")
        .body(Body::empty())
        .unwrap();
    let r = send(&app, hx).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers["hx-location"], "/login?next=%2Fdrop");

    // Browser 404 is a page; API 404 stays JSON.
    let r = send(&app, get("/nope", None)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("Nothing here"));
    let r = send(&app, get("/v1/nope", None)).await;
    assert!(r.body.starts_with('{'));

    // Static assets are served.
    for p in [
        "/static/app.css",
        "/static/app.js",
        "/static/recorder.js",
        "/static/vendor/htmx-2.0.10.min.js",
    ] {
        let r = send(&app, get(p, None)).await;
        assert_eq!(r.status, StatusCode::OK, "{p}");
    }
}

#[tokio::test]
async fn login_flow_does_not_burn_token_on_get() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));

    // Request: same "sent" answer, validation errors re-render with 400.
    let r = send(
        &app,
        post_form("/login", None, &[("email", "nope"), ("next", "/drop")]),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body.contains("Enter a valid email address."));
    let r = send(
        &app,
        post_form(
            "/login",
            None,
            &[("email", "w@x.com"), ("handle", "web_user")],
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Check your inbox"));
    assert!(r.body.contains("data-countdown=\"60\""));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM magic_tokens WHERE email='w@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Within the cooldown a resend says "sent" but issues no new token.
    let r = send(&app, post_form("/login", None, &[("email", "w@x.com")])).await;
    assert!(r.body.contains("Check your inbox"));
    let n2: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM magic_tokens WHERE email='w@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((n, n2), (1, 1));

    // GET with token only shows a button (prefetchers must not consume it).
    let token = questdrop::auth::issue_magic_token(&pool, "w2@x.com", None)
        .await
        .unwrap();
    let r = send(
        &app,
        get(&format!("/login?token={token}&next=/match"), None),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("action=\"/login/confirm\""));
    assert!(r.body.contains(&format!("value=\"{token}\"")));
    let used: bool = sqlx::query_scalar("SELECT used FROM magic_tokens WHERE email='w2@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!used);

    // Confirm with an unsafe next falls back to /match.
    let r = send(
        &app,
        post_form(
            "/login/confirm",
            None,
            &[("token", &token), ("next", "https://evil.example")],
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), "/match");
    // Second use: expired screen, no cookie.
    let r = send(
        &app,
        post_form("/login/confirm", None, &[("token", &token)]),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.body.contains("This link has expired."));
    assert!(r.headers.get(header::SET_COOKIE).is_none());

    // Logout clears the cookie.
    let ck = web_login(&app, &pool, "w3@x.com", "web_three").await;
    let r = send(&app, get("/me", Some(&ck))).await;
    assert!(r.body.contains("@web_three"));
    let r = send(&app, post_form("/logout", Some(&ck), &[])).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert!(r.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    let r = send(&app, get("/me", Some(&ck))).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "session gone");
}

#[tokio::test]
async fn csrf_cross_site_posts_are_refused() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));
    let ck = web_login(&app, &pool, "cs@x.com", "csrf_user").await;
    for site in ["cross-site", "same-site"] {
        let req = Request::builder()
            .method("POST")
            .uri("/me")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", format!("qd_session={ck}"))
            .header("sec-fetch-site", site)
            .body(Body::from("can_do=hacked"))
            .unwrap();
        let r = send(&app, req).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{site}");
    }
    let req = Request::builder()
        .method("POST")
        .uri("/me")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("cookie", format!("qd_session={ck}"))
        .header("host", "questdrop.example")
        .header("origin", "https://evil.example")
        .body(Body::from("can_do=hacked"))
        .unwrap();
    assert_eq!(send(&app, req).await.status, StatusCode::FORBIDDEN);
    let can: Vec<String> = sqlx::query_scalar("SELECT can_do FROM users WHERE email='cs@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(can.is_empty(), "nothing written");
}

#[tokio::test]
async fn drop_form_validates_saves_and_caps() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));
    let ck = web_login(&app, &pool, "dr@x.com", "drop_user").await;

    let r = send(&app, get("/drop", Some(&ck))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("enctype=\"multipart/form-data\""));
    assert!(
        !r.body.contains("name=\"email\""),
        "identity comes from the session"
    );

    // Empty form: every rule maps to its field; values are kept.
    let r = send(
        &app,
        post_multipart(
            "/drop",
            Some(&ck),
            &[("title", "Keep me"), ("link_url", "ftp://x")],
            None,
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    for msg in [
        "One sentence needs 10–200 characters.",
        "Use a link that starts with http:// or https://.",
        "Add a voice note",
        "Pick how long the next step would take.",
        "Pick how much energy it needs.",
    ] {
        assert!(r.body.contains(msg), "missing: {msg}");
    }
    assert!(r.body.contains("value=\"Keep me\""), "form data survives");
    assert!(
        !r.body.contains("one_liner must"),
        "raw server text never shown"
    );

    // HTMX gets 200 so the error page swaps in.
    let mut hx = post_multipart("/drop", Some(&ck), &[("title", "x")], None);
    hx.headers_mut()
        .insert("hx-request", "true".parse().unwrap());
    assert_eq!(send(&app, hx).await.status, StatusCode::OK);

    // No-JS path: the voice file posts with the form.
    let mut fields = drop_fields("With file");
    fields.retain(|(k, _)| *k != "voice_url");
    let mut mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    mp3.extend_from_slice(&[0u8; 64]);
    let r = send(
        &app,
        post_multipart("/drop", Some(&ck), &fields, Some(&mp3)),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let voice: String =
        sqlx::query_scalar("SELECT voice_url FROM projects WHERE title='With file'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(voice.starts_with("/voice/") && voice.ends_with(".mp3"));
    let skills: Vec<String> =
        sqlx::query_scalar("SELECT skill_needed FROM projects WHERE title='With file'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(skills, vec!["cli", "rust"]);
    // Not audio -> field error, nothing stored.
    let r = send(
        &app,
        post_multipart("/drop", Some(&ck), &fields, Some(b"hello this is text")),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body.contains("supported audio format"));

    // Success page, then the daily cap (3) blocks the form.
    let loc = web_drop(&app, &ck, "Second").await;
    let r = send(&app, get(&loc, Some(&ck))).await;
    assert!(r.body.contains("Your quest is in the pool"));
    assert!(r.body.contains("property=\"og:title\" content=\"Second\""));
    web_drop(&app, &ck, "Third").await;
    let r = send(
        &app,
        post_multipart("/drop", Some(&ck), &drop_fields("Fourth"), None),
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers.contains_key(header::RETRY_AFTER));
    assert!(
        r.body.contains("You&#x27;ve dropped 3 quests today")
            || r.body.contains("dropped 3 quests today")
    );
    let r = send(&app, get("/drop", Some(&ck))).await;
    assert!(
        r.body.contains("dropped 3 quests today"),
        "cap shown before filling in"
    );

    // `next` brings the user back (e.g. to /take/...).
    let ck2 = web_login(&app, &pool, "dr2@x.com", "drop_two").await;
    let mut f = drop_fields("Next test");
    f.retain(|(k, _)| *k != "next");
    f.push(("next", "/take/abc"));
    let r = send(&app, post_multipart("/drop", Some(&ck2), &f, None)).await;
    assert_eq!(r.location(), "/take/abc");
}

#[tokio::test]
async fn match_take_revive_loop() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));
    let ck_a = web_login(&app, &pool, "la@x.com", "loop_a").await;
    let ck_b = web_login(&app, &pool, "lb@x.com", "loop_b").await;
    web_drop(&app, &ck_a, "A quest").await;
    web_drop(&app, &ck_b, "B quest").await;

    // Pool shows offer-linked take buttons; own card says "Your quest".
    let r = send(&app, get("/", Some(&ck_a))).await;
    let offer_b = offer_of(&pool, "B quest").await;
    assert!(r.body.contains(&format!("href=\"/take/{offer_b}\"")));
    assert!(r.body.contains("Your quest"));

    // Match: B's quest with a human reason, never the raw "fit=".
    let r = send(&app, get("/match", Some(&ck_a))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("B quest") && !r.body.contains("A quest"));
    assert!(!r.body.contains("fit="));
    assert!(r.body.contains("Add your skills"), "no-skills hint");

    // Profile skills change the reason.
    let r = send(
        &app,
        post_form(
            "/me",
            Some(&ck_a),
            &[("can_do", "rust, CLI"), ("looking_for", "")],
        ),
    )
    .await;
    assert_eq!(r.location(), "/me?saved=1");
    let r = send(&app, get("/match", Some(&ck_a))).await;
    assert!(r.body.contains("Overlaps"), "{}", r.body);
    let r = send(
        &app,
        post_form("/me", Some(&ck_a), &[("can_do", &"x".repeat(40))]),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // Take screen: own open quest preselected, idempotency key embedded.
    let r = send(&app, get(&format!("/take/{offer_b}"), Some(&ck_a))).await;
    assert_eq!(r.status, StatusCode::OK);
    let offer_a = offer_of(&pool, "A quest").await;
    assert!(r
        .body
        .contains(&format!("value=\"{offer_a}\" required checked")));
    let idem = r
        .body
        .split("name=\"idem\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    // Own quest -> calm message, no form.
    let r = send(&app, get(&format!("/take/{offer_a}"), Some(&ck_a))).await;
    assert!(r.body.contains("This is your own quest") && !r.body.contains("name=\"idem\""));

    // Swap; a double submit with the same key replays the same match.
    let form = [("give_offer_id", offer_a.as_str()), ("idem", idem.as_str())];
    let r1 = send(
        &app,
        post_form(&format!("/take/{offer_b}"), Some(&ck_a), &form),
    )
    .await;
    let r2 = send(
        &app,
        post_form(&format!("/take/{offer_b}"), Some(&ck_a), &form),
    )
    .await;
    assert_eq!(r1.status, StatusCode::SEE_OTHER, "{}", r1.body);
    assert_eq!(r1.location(), r2.location(), "idempotent replay");
    assert!(r1.location().starts_with("/revive?swapped="));
    let matches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(matches, 1);

    // Revive: celebration, 4 localized steps naming a skill.
    let r = send(&app, get(r1.location(), Some(&ck_a))).await;
    assert!(r.body.contains("Swap done!"));
    assert!(r.body.contains("Run the smallest cli piece locally"));
    assert_eq!(r.body.matches("data-step=").count(), 4);
    assert!(r.body.contains("data-revive=\"questdrop:revive:"));
    // Given tab on the other side.
    let r = send(&app, get("/revive?tab=given", Some(&ck_b))).await;
    assert!(r.body.contains("Continuing with someone else"));

    // Taken quest is gone for others: a third user gets the friendly state.
    let ck_c = web_login(&app, &pool, "lc@x.com", "loop_c").await;
    web_drop(&app, &ck_c, "C quest").await;
    let r = send(&app, get(&format!("/take/{offer_b}"), Some(&ck_c))).await;
    assert!(r.body.contains("Someone took it before you"));
    let offer_c = offer_of(&pool, "C quest").await;
    let r = send(
        &app,
        post_form(
            &format!("/take/{offer_b}"),
            Some(&ck_c),
            &[("give_offer_id", &offer_c), ("idem", "k-c")],
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.body.contains("Someone took it before you"));

    // No open quest of your own -> "drop first" with a way back.
    let ck_d = web_login(&app, &pool, "ld@x.com", "loop_d").await;
    let r = send(&app, get(&format!("/take/{offer_c}"), Some(&ck_d))).await;
    assert!(r.body.contains(&format!("/drop?next=/take/{offer_c}")));

    // Quest page of a matched quest explains where it went.
    let pid: String = sqlx::query_scalar("SELECT id::text FROM projects WHERE title='B quest'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let r = send(&app, get(&format!("/q/{pid}"), None)).await;
    assert!(r.body.contains("This quest has a new owner"));
}

#[tokio::test]
async fn report_fragment_and_hide() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));
    let ck_g = web_login(&app, &pool, "rg@x.com", "rep_giver").await;
    web_drop(&app, &ck_g, "Reported").await;
    let pid: String = sqlx::query_scalar("SELECT id::text FROM projects WHERE title='Reported'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let hx = |ck: &str, reason: &str| {
        let mut r = post_form(
            &format!("/report/{pid}"),
            Some(ck),
            &[("reason", reason), ("back", "/")],
        );
        r.headers_mut()
            .insert("hx-request", "true".parse().unwrap());
        r
    };
    let mut cks = Vec::new();
    for i in 0..3 {
        cks.push(
            web_login(
                &app,
                &pool,
                &format!("r{i}@x.com"),
                &format!("reporter_{i}"),
            )
            .await,
        );
    }
    // Too short -> fragment with the form again (200 for HTMX).
    let r = send(&app, hx(&cks[0], "short")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Write 10–300 characters.") && !r.body.contains("<html"));
    let r = send(&app, hx(&cks[0], "This link points to nothing at all")).await;
    assert!(r.body.contains("Thanks, we&#x27;ve noted it.") || r.body.contains("noted it"));
    let r = send(&app, hx(&cks[0], "This link points to nothing at all")).await;
    assert!(r.body.contains("already reported"));
    // Reported card hides its menu for that user.
    let r = send(&app, get("/", Some(&cks[0]))).await;
    assert!(
        r.body.contains("You&#x27;ve already reported this")
            || r.body.contains("already reported this")
    );
    send(&app, hx(&cks[1], "This link points to nothing at all")).await;
    let r = send(&app, hx(&cks[2], "This link points to nothing at all")).await;
    assert!(
        r.body.contains("data-hide-card=\"card-"),
        "third report hides the card"
    );
    let r = send(&app, get("/", None)).await;
    assert!(!r.body.contains("Reported"));
    let r = send(&app, get(&format!("/q/{pid}"), None)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // Own quest: no report menu at all.
    let r = send(&app, get("/me", Some(&ck_g))).await;
    assert!(!r.body.contains("action=\"/report/"));
}

#[tokio::test]
async fn pool_pagination_fragment() {
    let _g = lock().await;
    let pool = test_pool().await;
    let app = build_app(Some(pool.clone()));
    // 22 quests from 8 users (3/day cap each).
    let mut n = 0;
    for u in 0..8 {
        let ck = web_login(&app, &pool, &format!("p{u}@x.com"), &format!("pager_{u}")).await;
        for _ in 0..3 {
            if n == 22 {
                break;
            }
            web_drop(&app, &ck, &format!("Quest {n:02}")).await;
            n += 1;
        }
    }
    let r = send(&app, get("/", None)).await;
    assert_eq!(r.body.matches("<article class=\"card").count(), 20);
    assert!(r.body.contains("Show more"));
    let cursor = r
        .body
        .split("/?cursor=")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let mut frag = get(&format!("/?cursor={cursor}&frag=1"), None);
    frag.headers_mut()
        .insert("hx-request", "true".parse().unwrap());
    let r = send(&app, frag).await;
    assert!(!r.body.contains("<html"), "fragment only");
    assert_eq!(r.body.matches("<article class=\"card").count(), 2);
    assert!(r.body.contains("id=\"pool-more\" hx-swap-oob=\"true\""));
    assert!(!r.body.contains("Show more"), "last page drops the button");
    // Without HTMX the same URL is a full page (no-JS paging).
    let r = send(&app, get(&format!("/?cursor={cursor}"), None)).await;
    assert!(r.body.contains("<html"));
}

#[tokio::test]
async fn db_down_pages_degrade_calmly() {
    let app = build_app(None);
    let r = send(&app, get("/", None)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.body.contains("We can&#x27;t reach the pool right now")
            || r.body.contains("reach the pool right now")
    );
    assert!(r.body.contains("The pool is empty."));
    let r = send(&app, get("/login", None)).await;
    assert_eq!(r.status, StatusCode::OK);
}
