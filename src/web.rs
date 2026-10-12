//! Server-rendered web UI (docs/web-ui.md). Askama pages, progressively
//! enhanced with HTMX (`hx-boost` on the body plus a few fragments).
//!
//! Rules that hold for every handler here:
//! - Every form works without JavaScript (plain POST + 303 redirect).
//! - Handlers call `db::` directly; the `/v1` REST API stays for bots.
//! - HTMX requests get 200 + a re-rendered page on errors (HTMX does not
//!   swap 4xx by default) and `HX-Location` instead of 303 redirects.
//! - Identity always comes from the session cookie, never from a form.
//! - POSTs pass a same-origin check (`Sec-Fetch-Site` / `Origin`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{ConnectInfo, Multipart, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Form,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    app::{self, AppState},
    auth,
    db::{self, DbError, PoolRow},
    i18n::{self, Lang},
    matcher::{self, Matcher, RuleMatcher},
    ratelimit::RateLimiter,
    validate,
};

pub const LANG_COOKIE: &str = "qd_lang";
const LOGIN_RESEND_SECS: i64 = 60;

// ---------------------------------------------------------------------------
// Page context
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Me {
    pub id: Uuid,
    pub handle: String,
}

/// Everything `base.html` needs: language, who is logged in, which nav
/// item is active, whether the database is reachable.
pub struct Ctx {
    pub lang: Lang,
    pub user: Option<Me>,
    pub nav: &'static str,
    pub db_down: bool,
    /// Current path + query, used for `next=` and the language switch.
    pub here: String,
    pub dev_mode: bool,
}

impl Ctx {
    pub fn t(&self, key: &str) -> &'static str {
        i18n::t(self.lang, key)
    }
    pub fn tf(&self, key: &str, arg: impl std::fmt::Display) -> String {
        i18n::tf(self.lang, key, arg)
    }
    pub fn lang_code(&self) -> &'static str {
        self.lang.code()
    }
    pub fn other_lang(&self) -> &'static str {
        self.lang.other().code()
    }
    pub fn lang_href(&self) -> String {
        format!("/lang/{}?next={}", self.other_lang(), enc(&self.here))
    }
    pub fn login_href(&self) -> String {
        if self.here == "/" || self.here.starts_with("/login") {
            "/login".into()
        } else {
            format!("/login?next={}", enc(&self.here))
        }
    }
    pub fn logged_in(&self) -> bool {
        self.user.is_some()
    }
    pub fn handle(&self) -> &str {
        self.user.as_ref().map(|u| u.handle.as_str()).unwrap_or("")
    }
    pub fn is(&self, nav: &str) -> bool {
        self.nav == nav
    }
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn lang_for(headers: &HeaderMap) -> Lang {
    if let Some(l) = app::cookie_value(headers, LANG_COOKIE).and_then(|v| Lang::from_code(&v)) {
        return l;
    }
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .and_then(Lang::from_accept_language)
        .unwrap_or(Lang::En)
}

fn dev_mode() -> bool {
    let smtp = std::env::var("SMTP_URL").unwrap_or_default();
    let base = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    smtp.trim().is_empty() && (base.contains("://localhost") || base.contains("://127.0.0.1"))
}

async fn ctx(s: &AppState, headers: &HeaderMap, uri: &axum::http::Uri, nav: &'static str) -> Ctx {
    let here = uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let mut c = Ctx {
        lang: lang_for(headers),
        user: None,
        nav,
        db_down: s.pool.is_none(),
        here,
        dev_mode: dev_mode(),
    };
    if let (Some(pool), Some(cookie)) = (&s.pool, app::session_cookie(headers)) {
        match auth::load_session_user(pool, &s.session_secret, &cookie).await {
            Ok(Some(uid)) => match db::user_info(pool, uid).await {
                Ok(Some(u)) => {
                    c.user = Some(Me {
                        id: u.id,
                        handle: u.handle,
                    })
                }
                Ok(None) => {}
                Err(_) => c.db_down = true,
            },
            Ok(None) => {}
            Err(_) => c.db_down = true,
        }
    }
    c
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn is_htmx(h: &HeaderMap) -> bool {
    h.contains_key("hx-request")
}

/// HTMX only swaps 2xx by default: keep the real status for plain
/// browsers (and tests), answer 200 to HTMX so the error page renders.
fn status_for(h: &HeaderMap, status: StatusCode) -> StatusCode {
    if is_htmx(h) && status.is_client_error() {
        StatusCode::OK
    } else {
        status
    }
}

fn render<T: Template>(h: &HeaderMap, status: StatusCode, t: T) -> Response {
    match t.render() {
        Ok(body) => {
            let mut res = (status_for(h, status), body).into_response();
            res.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            // Pages are per-user; never let a shared cache keep them.
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            res.headers_mut()
                .insert(header::VARY, HeaderValue::from_static("Cookie, HX-Request"));
            res
        }
        Err(e) => {
            tracing::error!(error = %e, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "render failed").into_response()
        }
    }
}

/// 303 for browsers, `HX-Location` (ajax navigation + history push) for HTMX.
fn redirect(h: &HeaderMap, to: &str) -> Response {
    let to = HeaderValue::from_str(to).unwrap_or_else(|_| HeaderValue::from_static("/"));
    if is_htmx(h) {
        let mut res = StatusCode::OK.into_response();
        res.headers_mut()
            .insert(axum::http::HeaderName::from_static("hx-location"), to);
        res
    } else {
        let mut res = StatusCode::SEE_OTHER.into_response();
        res.headers_mut().insert(header::LOCATION, to);
        res
    }
}

/// Only same-site relative paths: "/x" yes; "//evil", "/\evil", "https://" no.
pub fn safe_next(raw: Option<&str>) -> Option<String> {
    let n = raw?.trim();
    let ok = n.starts_with('/')
        && !n.starts_with("//")
        && !n.contains('\\')
        && n.len() <= 512
        && !n.chars().any(|c| c.is_control());
    ok.then(|| n.to_string())
}

/// CSRF defence next to SameSite=Lax (web-ui §9): modern browsers send
/// `Sec-Fetch-Site`; older ones send `Origin` on POST. A request with
/// neither cannot come from a cross-site browser form, so it passes.
pub fn same_origin(h: &HeaderMap) -> bool {
    if let Some(site) = h.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        return matches!(site, "same-origin" | "none");
    }
    let Some(origin) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = h.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    match url::Url::parse(origin) {
        Ok(u) => {
            let o = match (u.host_str(), u.port()) {
                (Some(hh), Some(p)) => format!("{hh}:{p}"),
                (Some(hh), None) => hh.to_string(),
                _ => return false,
            };
            o.eq_ignore_ascii_case(host)
        }
        Err(_) => false,
    }
}

fn csrf_page(c: Ctx, h: &HeaderMap) -> Response {
    message(
        h,
        c,
        StatusCode::FORBIDDEN,
        "common.error_generic",
        "common.csrf",
        "/",
        "common.back_to_pool",
    )
}

#[derive(Template)]
#[template(path = "message.html")]
struct MessageTpl {
    c: Ctx,
    heading: &'static str,
    body: String,
    href: String,
    label: &'static str,
}

fn message(
    h: &HeaderMap,
    c: Ctx,
    status: StatusCode,
    heading: &str,
    body: &str,
    href: &str,
    label: &str,
) -> Response {
    let tpl = MessageTpl {
        heading: c.t(heading),
        body: c.t(body).to_string(),
        href: href.to_string(),
        label: c.t(label),
        c,
    };
    render(h, status, tpl)
}

/// HTML 404 for browser navigations (the JSON 404 stays for API clients).
pub async fn not_found_page(s: &AppState, h: &HeaderMap, uri: &axum::http::Uri) -> Response {
    let c = ctx(s, h, uri, "").await;
    message(
        h,
        c,
        StatusCode::NOT_FOUND,
        "common.not_found_title",
        "common.not_found_body",
        "/",
        "common.back_to_pool",
    )
}

fn login_redirect(h: &HeaderMap, c: &Ctx) -> Response {
    redirect(h, &c.login_href())
}

// ---------------------------------------------------------------------------
// Rate limiting for UI forms (HTML 429 instead of JSON)
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "message.html")]
struct TooManyTpl {
    c: Ctx,
    heading: &'static str,
    body: String,
    href: String,
    label: &'static str,
}

pub async fn ui_limit(
    State(lim): State<Arc<RateLimiter>>,
    conn: Option<ConnectInfo<std::net::SocketAddr>>,
    req: Request,
    next: Next,
) -> Response {
    let ip = app::client_ip(&req, conn);
    if let Some(retry) = lim.check(ip) {
        let h = req.headers().clone();
        let lang = lang_for(&h);
        let c = Ctx {
            lang,
            user: None,
            nav: "",
            db_down: false,
            here: req.uri().path().to_string(),
            dev_mode: false,
        };
        let tpl = TooManyTpl {
            heading: c.t("common.error_generic"),
            body: c.tf("common.too_many", retry),
            href: req.uri().path().to_string(),
            label: c.t("common.retry"),
            c,
        };
        let mut res = render(&h, StatusCode::TOO_MANY_REQUESTS, tpl);
        res.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(retry));
        return res;
    }
    next.run(req).await
}

// ---------------------------------------------------------------------------
// Quest cards
// ---------------------------------------------------------------------------

/// Everything `card.html` renders, precomputed so templates stay dumb.
#[derive(Clone)]
pub struct CardView {
    pub id: String,
    pub title: String,
    pub one_liner: String,
    pub link_url: String,
    pub voice_url: String,
    pub voice_external: bool,
    pub skills: Vec<String>,
    pub tb: String,
    pub en: String,
    pub giver: String,
    pub offer_id: Option<String>,
    /// Human match reason ("Overlaps 50% with your skills").
    pub reason: Option<String>,
    pub surprise: bool,
    pub own: bool,
    /// "login" | "can" | "done" | "none"
    pub report: &'static str,
    /// Show the primary "Take this" action.
    pub action: bool,
    /// Heading level for the title (2 on single-quest pages, 3 in lists).
    pub h: u8,
    pub tb_label: &'static str,
    pub tb_help: &'static str,
    pub en_label: &'static str,
    pub en_long: &'static str,
}

impl CardView {
    fn new(row: &PoolRow, me: Option<&Me>, reported: &HashSet<Uuid>, lang: Lang) -> Self {
        let own = me.is_some_and(|m| m.handle == row.giver_handle);
        let report = match me {
            _ if own => "none",
            None => "login",
            Some(_) if reported.contains(&row.id) => "done",
            Some(_) => "can",
        };
        CardView {
            id: row.id.to_string(),
            title: row.title.clone(),
            one_liner: row.one_liner.clone(),
            link_url: row.link_url.clone(),
            voice_url: row.voice_url.clone(),
            voice_external: !row.voice_url.starts_with("/voice/"),
            skills: row.skill_needed.clone(),
            tb: row.time_bucket.clone(),
            en: row.energy.clone(),
            giver: row.giver_handle.clone(),
            offer_id: row.offer_id.map(|o| o.to_string()),
            reason: None,
            surprise: false,
            own,
            report,
            action: true,
            h: 3,
            tb_label: i18n::t(lang, &format!("time.{}", row.time_bucket)),
            tb_help: i18n::t(lang, &format!("time.{}_help", row.time_bucket)),
            en_label: i18n::t(lang, &format!("energy.{}_short", row.energy)),
            en_long: i18n::t(lang, &format!("energy.{}", row.energy)),
        }
    }

    pub fn can_take(&self) -> bool {
        self.action && !self.own && self.offer_id.is_some()
    }
    pub fn take_href(&self) -> String {
        format!("/take/{}", self.offer_id.as_deref().unwrap_or(""))
    }
    pub fn energy_level(&self) -> u8 {
        match self.en.as_str() {
            "LOW" => 1,
            "MID" => 2,
            _ => 3,
        }
    }
}

async fn reported_set(s: &AppState, me: Option<&Me>) -> HashSet<Uuid> {
    match (&s.pool, me) {
        (Some(pool), Some(m)) => db::reported_ids(pool, m.id)
            .await
            .map(|v| v.into_iter().collect())
            .unwrap_or_default(),
        _ => HashSet::new(),
    }
}

// ---------------------------------------------------------------------------
// Pool  (/)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct PoolQ {
    cursor: Option<String>,
    frag: Option<String>,
}

#[derive(Template)]
#[template(path = "pool.html")]
struct PoolTpl {
    c: Ctx,
    cards: Vec<CardView>,
    next_cursor: Option<String>,
    count_text: String,
    paged: bool,
}

#[derive(Template)]
#[template(path = "pool_frag.html")]
struct PoolFragTpl {
    c: Ctx,
    cards: Vec<CardView>,
    next_cursor: Option<String>,
}

pub async fn pool_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    q: Option<Query<PoolQ>>,
) -> Response {
    let mut c = ctx(&s, &headers, &uri, "pool").await;
    let q = q.map(|Query(q)| q).unwrap_or_default();
    let cursor = q.cursor.as_deref().and_then(|v| v.parse::<Uuid>().ok());
    let reported = reported_set(&s, c.user.as_ref()).await;
    let (rows, next, count) = match &s.pool {
        Some(pool) => match (
            db::list_pool(pool, cursor, 20).await,
            db::count_open(pool).await,
        ) {
            (Ok((rows, next)), Ok(n)) => (rows, next, n),
            _ => {
                c.db_down = true;
                (Vec::new(), None, 0)
            }
        },
        None => (Vec::new(), None, 0),
    };
    let cards: Vec<CardView> = rows
        .iter()
        .map(|r| CardView::new(r, c.user.as_ref(), &reported, c.lang))
        .collect();
    let next_cursor = next.map(|n| n.to_string());
    if is_htmx(&headers) && q.frag.is_some() {
        return render(
            &headers,
            StatusCode::OK,
            PoolFragTpl {
                c,
                cards,
                next_cursor,
            },
        );
    }
    let count_text = if count == 1 {
        c.t("pool.count_one").to_string()
    } else {
        c.tf("pool.count", count)
    };
    render(
        &headers,
        StatusCode::OK,
        PoolTpl {
            c,
            cards,
            next_cursor,
            count_text,
            paged: cursor.is_some(),
        },
    )
}

// ---------------------------------------------------------------------------
// Language switch
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct NextQ {
    next: Option<String>,
}

pub async fn set_lang(Path(code): Path<String>, Query(q): Query<NextQ>) -> Response {
    let lang = Lang::from_code(&code).unwrap_or(Lang::En);
    let to = safe_next(q.next.as_deref()).unwrap_or_else(|| "/".into());
    let mut res = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(&to) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    let cookie = format!(
        "{LANG_COOKIE}={}; Path=/; Max-Age=31536000; SameSite=Lax",
        lang.code()
    );
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTpl {
    c: Ctx,
    /// "form" | "sent" | "confirm" | "expired"
    mode: &'static str,
    email: String,
    handle: String,
    next: String,
    token: String,
    wait: i64,
    err_email: Option<&'static str>,
    err_handle: Option<&'static str>,
    need: bool,
}

impl LoginTpl {
    fn new(c: Ctx, mode: &'static str, next: String) -> Self {
        LoginTpl {
            c,
            mode,
            email: String::new(),
            handle: String::new(),
            next,
            token: String::new(),
            wait: 0,
            err_email: None,
            err_handle: None,
            need: false,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct LoginQ {
    token: Option<String>,
    next: Option<String>,
}

pub async fn login_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    q: Option<Query<LoginQ>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "login").await;
    let q = q.map(|Query(q)| q).unwrap_or_default();
    let next = safe_next(q.next.as_deref()).unwrap_or_default();
    // The token is NOT consumed on GET: mail clients prefetch links, and a
    // prefetch must not burn the one-time token. A button POSTs it.
    if let Some(token) = q.token.filter(|t| !t.trim().is_empty()) {
        let mut tpl = LoginTpl::new(c, "confirm", next);
        tpl.token = token;
        return render(&headers, StatusCode::OK, tpl);
    }
    if c.logged_in() {
        let to = if next.is_empty() {
            "/match".to_string()
        } else {
            next
        };
        return redirect(&headers, &to);
    }
    let mut tpl = LoginTpl::new(c, "form", next.clone());
    tpl.need = !next.is_empty();
    render(&headers, StatusCode::OK, tpl)
}

#[derive(Deserialize, Default)]
pub struct LoginForm {
    #[serde(default)]
    email: String,
    #[serde(default)]
    handle: String,
    #[serde(default)]
    next: String,
}

pub async fn login_post(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    form: Option<Form<LoginForm>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "login").await;
    if !same_origin(&headers) {
        return csrf_page(c, &headers);
    }
    let f = form.map(|Form(f)| f).unwrap_or_default();
    let next = safe_next(Some(&f.next)).unwrap_or_default();
    let email = f.email.trim().to_lowercase();
    let handle = f.handle.trim().to_string();
    let mut tpl = LoginTpl::new(c, "form", next.clone());
    tpl.email = email.clone();
    tpl.handle = handle.clone();
    if !validate::valid_email(&email) {
        tpl.err_email = Some(tpl.c.t("login.err_email"));
    }
    if !handle.is_empty() && !validate::valid_handle(&handle) {
        tpl.err_handle = Some(tpl.c.t("login.err_handle"));
    }
    if tpl.err_email.is_some() || tpl.err_handle.is_some() {
        return render(&headers, StatusCode::BAD_REQUEST, tpl);
    }
    let Some(pool) = &s.pool else {
        tpl.c.db_down = true;
        return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
    };
    // Per-email resend cooldown. The answer is "sent" either way, so the
    // page never reveals whether an account exists.
    let wait = match db::login_cooldown(pool, &email, LOGIN_RESEND_SECS).await {
        Ok(w) => w,
        Err(_) => {
            tpl.c.db_down = true;
            return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
        }
    };
    if wait == 0 {
        let hint = (!handle.is_empty()).then_some(handle.as_str());
        match auth::issue_magic_token(pool, &email, hint).await {
            Ok(token) => {
                let link = auth::login_link_next(&token, (!next.is_empty()).then_some(&next));
                if let Err(e) = s.mailer.send_login_link(&email, &link).await {
                    tracing::error!(error = %e, "mailer failed");
                }
            }
            Err(_) => {
                tpl.c.db_down = true;
                return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
            }
        }
    }
    tpl.mode = "sent";
    tpl.wait = if wait == 0 { LOGIN_RESEND_SECS } else { wait };
    render(&headers, StatusCode::OK, tpl)
}

#[derive(Deserialize, Default)]
pub struct ConfirmForm {
    #[serde(default)]
    token: String,
    #[serde(default)]
    next: String,
}

pub async fn login_confirm(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    form: Option<Form<ConfirmForm>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "login").await;
    if !same_origin(&headers) {
        return csrf_page(c, &headers);
    }
    let f = form.map(|Form(f)| f).unwrap_or_default();
    let next = safe_next(Some(&f.next)).unwrap_or_default();
    let Some(pool) = &s.pool else {
        let mut tpl = LoginTpl::new(c, "confirm", next);
        tpl.token = f.token;
        tpl.c.db_down = true;
        return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
    };
    let user = match auth::consume_magic_token(pool, f.token.trim()).await {
        Ok(u) => u,
        Err(auth::AuthError::Invalid) => {
            let tpl = LoginTpl::new(c, "expired", next);
            return render(&headers, StatusCode::UNAUTHORIZED, tpl);
        }
        Err(auth::AuthError::Db(_)) => {
            let mut tpl = LoginTpl::new(c, "confirm", next);
            tpl.token = f.token;
            tpl.c.db_down = true;
            return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
        }
    };
    let value = match auth::create_session(pool, &s.session_secret, user).await {
        Ok(v) => v,
        Err(_) => {
            let mut tpl = LoginTpl::new(c, "confirm", next);
            tpl.token = f.token;
            tpl.c.db_down = true;
            return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
        }
    };
    let to = if next.is_empty() {
        "/match".to_string()
    } else {
        next
    };
    let mut res = redirect(&headers, &to);
    if let Ok(v) = HeaderValue::from_str(&auth::set_cookie_header(&value, false)) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
    res
}

pub async fn logout_post(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    if !same_origin(&headers) {
        let c = ctx(&s, &headers, &uri, "").await;
        return csrf_page(c, &headers);
    }
    if let (Some(pool), Some(cookie)) = (&s.pool, app::session_cookie(&headers)) {
        let _ = auth::destroy_session(pool, &s.session_secret, &cookie).await;
    }
    let mut res = redirect(&headers, "/");
    if let Ok(v) = HeaderValue::from_str(&auth::set_cookie_header("dead", true)) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
    res
}

// ---------------------------------------------------------------------------
// Drop
// ---------------------------------------------------------------------------

pub struct Opt {
    pub value: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub checked: bool,
    pub level: u8,
}

#[derive(Template)]
#[template(path = "drop.html")]
struct DropTpl {
    c: Ctx,
    title: String,
    one_liner: String,
    link_url: String,
    voice_url: String,
    skills: String,
    next: String,
    time_opts: Vec<Opt>,
    energy_opts: Vec<Opt>,
    summary: Vec<&'static str>,
    e_title: Option<&'static str>,
    e_one_liner: Option<&'static str>,
    e_link: Option<&'static str>,
    e_voice: Option<&'static str>,
    e_time: Option<&'static str>,
    e_energy: Option<&'static str>,
    e_skills: Option<&'static str>,
    cap: Option<String>,
    cap_secs: u64,
}

#[derive(Default)]
struct DropForm {
    title: String,
    one_liner: String,
    link_url: String,
    voice_url: String,
    skills: String,
    time_bucket: String,
    energy: String,
    next: String,
}

impl DropTpl {
    fn new(c: Ctx, f: &DropForm) -> Self {
        let time_opts = ["S", "M", "L"]
            .iter()
            .enumerate()
            .map(|(i, v)| Opt {
                value: v,
                label: c.t(&format!("time.{v}")),
                help: c.t(&format!("time.{v}_help")),
                checked: f.time_bucket == *v,
                level: i as u8 + 1,
            })
            .collect();
        let energy_opts = ["LOW", "MID", "HIGH"]
            .iter()
            .enumerate()
            .map(|(i, v)| Opt {
                value: v,
                label: c.t(&format!("energy.{v}_short")),
                help: c.t(&format!("energy.{v}")),
                checked: f.energy == *v,
                level: i as u8 + 1,
            })
            .collect();
        DropTpl {
            c,
            title: f.title.clone(),
            one_liner: f.one_liner.clone(),
            link_url: f.link_url.clone(),
            voice_url: f.voice_url.clone(),
            skills: f.skills.clone(),
            next: f.next.clone(),
            time_opts,
            energy_opts,
            summary: Vec::new(),
            e_title: None,
            e_one_liner: None,
            e_link: None,
            e_voice: None,
            e_time: None,
            e_energy: None,
            e_skills: None,
            cap: None,
            cap_secs: 0,
        }
    }

    /// Server validation messages are English and stable; map each to its
    /// field and a translated sentence. Unknown ones go to the summary only.
    fn apply_errors(&mut self, errs: &[String]) {
        for e in errs {
            let (slot, key) = if e.starts_with("title") {
                (&mut self.e_title, "drop.err_title")
            } else if e.starts_with("one_liner") {
                (&mut self.e_one_liner, "drop.err_one_liner")
            } else if e.starts_with("link_url") {
                (&mut self.e_link, "drop.err_link")
            } else if e.starts_with("voice file not found") {
                (&mut self.e_voice, "drop.err_voice_missing")
            } else if e.starts_with("voice") {
                (&mut self.e_voice, "drop.err_voice")
            } else if e.starts_with("time_bucket") {
                (&mut self.e_time, "drop.err_time")
            } else if e.starts_with("energy") {
                (&mut self.e_energy, "drop.err_energy")
            } else if e.starts_with("each skill") {
                (&mut self.e_skills, "drop.err_skill")
            } else {
                self.summary.push(self.c.t("common.error_generic"));
                continue;
            };
            let msg = self.c.t(key);
            *slot = Some(msg);
            self.summary.push(msg);
        }
    }

    fn set_cap(&mut self, retry: u64) {
        let span = if retry >= 3600 {
            self.c.tf("drop.hours", retry.div_ceil(3600))
        } else {
            self.c.tf("drop.minutes", retry.div_ceil(60).max(1))
        };
        self.cap = Some(self.c.tf("drop.cap", span));
        self.cap_secs = retry;
    }
}

pub async fn drop_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    q: Option<Query<NextQ>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "drop").await;
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let next = q
        .and_then(|Query(q)| safe_next(q.next.as_deref()))
        .unwrap_or_default();
    let f = DropForm {
        next,
        ..Default::default()
    };
    let mut tpl = DropTpl::new(c, &f);
    if let Some(pool) = &s.pool {
        if let Ok(Some(retry)) = db::daily_cap_retry(pool, me.id).await {
            tpl.set_cap(retry);
        }
    }
    render(&headers, StatusCode::OK, tpl)
}

pub async fn drop_post(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    mp: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "drop").await;
    if !same_origin(&headers) {
        return csrf_page(c, &headers);
    }
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let mut f = DropForm::default();
    let mut file: Vec<u8> = Vec::new();
    let mut too_big = false;
    if let Ok(mut mp) = mp {
        while let Ok(Some(mut field)) = mp.next_field().await {
            let name = field.name().unwrap_or("").to_string();
            if name == "voice_file" {
                while let Ok(Some(chunk)) = field.chunk().await {
                    if file.len() + chunk.len() > app::VOICE_MAX_BYTES {
                        too_big = true;
                        break;
                    }
                    file.extend_from_slice(&chunk);
                }
                if too_big {
                    break;
                }
                continue;
            }
            let v = field.text().await.unwrap_or_default();
            match name.as_str() {
                "title" => f.title = v,
                "one_liner" => f.one_liner = v,
                "link_url" => f.link_url = v,
                "voice_url" => f.voice_url = v.trim().to_string(),
                "skills" => f.skills = v,
                "time_bucket" => f.time_bucket = v,
                "energy" => f.energy = v,
                "next" => f.next = safe_next(Some(&v)).unwrap_or_default(),
                _ => {}
            }
        }
    }
    let mut tpl = DropTpl::new(c, &f);
    let Some(pool) = &s.pool else {
        tpl.c.db_down = true;
        return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
    };
    // No-JS path: the file arrives with the form. With JS the recorder has
    // already uploaded it and only voice_url is sent.
    if too_big {
        tpl.e_voice = Some(tpl.c.t("rec.too_big"));
        tpl.summary.push(tpl.c.t("rec.too_big"));
        return render(&headers, StatusCode::PAYLOAD_TOO_LARGE, tpl);
    }
    if f.voice_url.is_empty() && !file.is_empty() {
        match app::save_voice(&s.voice_dir, &file).await {
            Ok(url) => {
                f.voice_url = url.clone();
                tpl.voice_url = url;
            }
            Err(e) => {
                let (status, key) = match e {
                    app::VoiceError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "rec.too_big"),
                    app::VoiceError::NotAudio => (StatusCode::BAD_REQUEST, "rec.bad_type"),
                    app::VoiceError::Full => (StatusCode::INSUFFICIENT_STORAGE, "rec.full"),
                    app::VoiceError::Storage => (StatusCode::SERVICE_UNAVAILABLE, "rec.failed"),
                };
                tpl.e_voice = Some(tpl.c.t(key));
                tpl.summary.push(tpl.c.t(key));
                return render(&headers, status, tpl);
            }
        }
    }
    let info = match db::user_info(pool, me.id).await {
        Ok(Some(u)) => u,
        Ok(None) => return login_redirect(&headers, &tpl.c),
        Err(_) => {
            tpl.c.db_down = true;
            return render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl);
        }
    };
    let raw = validate::RawDrop {
        handle: info.handle,
        email: info.email,
        title: f.title.clone(),
        one_liner: f.one_liner.clone(),
        link_url: f.link_url.clone(),
        voice_url: f.voice_url.clone(),
        skill_needed: split_list(&f.skills),
        time_bucket: f.time_bucket.clone(),
        energy: f.energy.clone(),
    };
    let mut errs = match validate::validate_drop(&raw) {
        Ok(_) => Vec::new(),
        Err(e) => e,
    };
    if errs.is_empty()
        && raw.voice_url.starts_with("/voice/")
        && !app::voice_file_exists(&s.voice_dir, raw.voice_url.trim())
    {
        errs.push("voice file not found under /voice/".into());
        tpl.voice_url.clear();
    }
    if !errs.is_empty() {
        tpl.apply_errors(&errs);
        return render(&headers, StatusCode::BAD_REQUEST, tpl);
    }
    let valid = match validate::validate_drop(&raw) {
        Ok(v) => v,
        Err(_) => return render(&headers, StatusCode::BAD_REQUEST, tpl),
    };
    match db::create_drop_as(pool, me.id, &valid).await {
        Ok((project_id, _)) => {
            let to = if f.next.is_empty() {
                format!("/q/{project_id}?dropped=1")
            } else {
                f.next.clone()
            };
            redirect(&headers, &to)
        }
        Err(DbError::DailyCap(retry)) => {
            tpl.set_cap(retry);
            let mut res = render(&headers, StatusCode::TOO_MANY_REQUESTS, tpl);
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry));
            res
        }
        Err(DbError::Db(e)) => {
            tracing::error!(error = %e, "drop failed");
            tpl.c.db_down = true;
            render(&headers, StatusCode::SERVICE_UNAVAILABLE, tpl)
        }
        Err(_) => {
            tpl.summary.push(tpl.c.t("common.error_generic"));
            render(&headers, StatusCode::BAD_REQUEST, tpl)
        }
    }
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Match
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "match.html")]
struct MatchTpl {
    c: Ctx,
    cards: Vec<CardView>,
    no_skills: bool,
}

pub async fn match_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let mut c = ctx(&s, &headers, &uri, "match").await;
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let Some(pool) = &s.pool else {
        return render(
            &headers,
            StatusCode::OK,
            MatchTpl {
                c,
                cards: Vec::new(),
                no_skills: false,
            },
        );
    };
    let loaded = async {
        let user = db::load_user(pool, me.id)
            .await?
            .ok_or(DbError::NotFound("user".into()))?;
        let items = db::match_candidates(pool, me.id).await?;
        let ranked = RuleMatcher.match_items(&user, &items);
        let ids: Vec<Uuid> = ranked.iter().filter_map(|r| r.id.parse().ok()).collect();
        let rows = db::projects_by_ids(pool, &ids).await?;
        Ok::<_, DbError>((user, ranked, rows))
    }
    .await;
    let (user, ranked, rows) = match loaded {
        Ok(v) => v,
        Err(_) => {
            c.db_down = true;
            return render(
                &headers,
                StatusCode::OK,
                MatchTpl {
                    c,
                    cards: Vec::new(),
                    no_skills: false,
                },
            );
        }
    };
    let no_skills = user.can_do.is_empty() && user.looking_for.is_empty();
    let reported = reported_set(&s, Some(&me)).await;
    let by_id: HashMap<String, &PoolRow> = rows.iter().map(|r| (r.id.to_string(), r)).collect();
    let mut cards = Vec::new();
    for (i, r) in ranked.iter().enumerate() {
        let Some(row) = by_id.get(&r.id) else {
            continue;
        };
        let mut card = CardView::new(row, Some(&me), &reported, c.lang);
        // The matcher returns top-3 + one random "surprise" from the rest.
        card.surprise = i == 3;
        let fit = matcher::fit_score(
            &user,
            &matcher::PoolItem {
                id: String::new(),
                offer_id: String::new(),
                skill_needed: row.skill_needed.clone(),
                time_bucket: String::new(),
                energy: String::new(),
            },
        );
        card.reason = Some(if card.surprise {
            c.t("match.surprise_reason").to_string()
        } else if fit > 0.0 {
            c.tf("match.fit_pct", (fit * 100.0).round() as i64)
        } else {
            c.t("match.fit_none").to_string()
        });
        cards.push(card);
    }
    render(
        &headers,
        StatusCode::OK,
        MatchTpl {
            c,
            cards,
            no_skills,
        },
    )
}

// ---------------------------------------------------------------------------
// Take (swap confirmation)
// ---------------------------------------------------------------------------

pub struct GiveOpt {
    pub offer_id: String,
    pub title: String,
    pub one_liner: String,
    pub checked: bool,
}

#[derive(Template)]
#[template(path = "take.html")]
struct TakeTpl {
    c: Ctx,
    /// "ok" | "gone" | "own" | "none" | "notfound"
    state: &'static str,
    card: Option<CardView>,
    mine: Vec<GiveOpt>,
    offer_id: String,
    idem: String,
    error: Option<&'static str>,
}

async fn take_tpl(
    s: &AppState,
    c: Ctx,
    me: &Me,
    offer: Uuid,
    picked: Option<&str>,
) -> Result<TakeTpl, Ctx> {
    let pool = match &s.pool {
        Some(p) => p,
        None => return Err(c),
    };
    let target = match db::project_for_offer(pool, offer).await {
        Ok(t) => t,
        Err(_) => return Err(c),
    };
    let mut tpl = TakeTpl {
        c,
        state: "ok",
        card: None,
        mine: Vec::new(),
        offer_id: offer.to_string(),
        idem: Uuid::new_v4().to_string(),
        error: None,
    };
    let Some(p) = target else {
        tpl.state = "notfound";
        return Ok(tpl);
    };
    if p.status == "CLOSED" {
        tpl.state = "notfound";
        return Ok(tpl);
    }
    let reported = reported_set(s, Some(me)).await;
    let mut card = CardView::new(&p.row, Some(me), &reported, tpl.c.lang);
    card.action = false;
    card.h = 2;
    if p.giver_id == me.id {
        tpl.state = "own";
    } else if p.status != "OPEN" || p.row.offer_id != Some(offer) {
        tpl.state = "gone";
    }
    tpl.card = Some(card);
    if tpl.state != "ok" {
        return Ok(tpl);
    }
    let mine = match db::my_quests(pool, me.id).await {
        Ok(m) => m,
        Err(_) => return Err(tpl.c),
    };
    tpl.mine = mine
        .into_iter()
        .filter(|q| q.status == "OPEN")
        .filter_map(|q| {
            let oid = q.row.offer_id?.to_string();
            Some(GiveOpt {
                checked: picked == Some(oid.as_str()),
                offer_id: oid,
                title: q.row.title,
                one_liner: q.row.one_liner,
            })
        })
        .collect();
    if tpl.mine.len() == 1 {
        tpl.mine[0].checked = true;
    }
    if tpl.mine.is_empty() {
        tpl.state = "none";
    }
    Ok(tpl)
}

fn db_down_page(h: &HeaderMap, mut c: Ctx) -> Response {
    c.db_down = true;
    message(
        h,
        c,
        StatusCode::SERVICE_UNAVAILABLE,
        "common.error_generic",
        "common.db_down",
        "/",
        "common.back_to_pool",
    )
}

pub async fn take_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(offer): Path<String>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "match").await;
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let Ok(offer) = offer.parse::<Uuid>() else {
        return not_found_page(&s, &headers, &uri).await;
    };
    match take_tpl(&s, c, &me, offer, None).await {
        Ok(tpl) => {
            let status = if tpl.state == "notfound" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            };
            render(&headers, status, tpl)
        }
        Err(c) => db_down_page(&headers, c),
    }
}

#[derive(Deserialize, Default)]
pub struct TakeForm {
    #[serde(default)]
    give_offer_id: String,
    #[serde(default)]
    idem: String,
}

pub async fn take_post(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(offer): Path<String>,
    form: Option<Form<TakeForm>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "match").await;
    if !same_origin(&headers) {
        return csrf_page(c, &headers);
    }
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let Ok(take) = offer.parse::<Uuid>() else {
        return not_found_page(&s, &headers, &uri).await;
    };
    let Some(pool) = s.pool.clone() else {
        return db_down_page(&headers, c);
    };
    let f = form.map(|Form(f)| f).unwrap_or_default();
    let give = f.give_offer_id.parse::<Uuid>().ok();
    // Idempotency-Key minted at render time (web-ui §7.3): a double submit
    // replays the stored result (200) instead of failing.
    let key = Some(f.idem.trim())
        .filter(|k| !k.is_empty() && k.len() <= 64)
        .map(str::to_string);
    let error: &'static str = match give {
        None => "take.pick",
        Some(give) => match db::create_swap(&pool, me.id, give, take, key.as_deref()).await {
            Ok((_, body)) => {
                return redirect(
                    &headers,
                    &format!("/revive?swapped={}#m-{}", body.match_id, body.match_id),
                );
            }
            Err(DbError::Conflict(d)) if d.contains("no longer open") => "take.gone",
            Err(DbError::Conflict(d)) if d.contains("not yours") => "take.not_yours",
            Err(DbError::Conflict(d)) if d.contains("own project") => "take.own",
            Err(DbError::NotFound(_)) => "take.not_found",
            Err(DbError::Db(e)) => {
                tracing::error!(error = %e, "swap failed");
                return db_down_page(&headers, c);
            }
            Err(_) => "common.error_generic",
        },
    };
    match take_tpl(&s, c, &me, take, Some(&f.give_offer_id)).await {
        Ok(mut tpl) => {
            let status = if error == "take.gone" {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            // A lost race shows the friendly "someone was faster" state.
            if error != "take.gone" || tpl.state == "ok" {
                tpl.error = Some(tpl.c.t(error));
            }
            render(&headers, status, tpl)
        }
        Err(c) => db_down_page(&headers, c),
    }
}

// ---------------------------------------------------------------------------
// Revive / my quests
// ---------------------------------------------------------------------------

pub struct ReviveView {
    pub match_id: String,
    pub taken_on: String,
    pub card: CardView,
    pub steps: Vec<String>,
    pub open: bool,
}

pub struct GivenView {
    pub card: CardView,
    pub status: String,
    pub status_label: &'static str,
}

#[derive(Template)]
#[template(path = "revive.html")]
struct ReviveTpl {
    c: Ctx,
    tab: &'static str,
    revives: Vec<ReviveView>,
    given: Vec<GivenView>,
    swapped: bool,
}

#[derive(Deserialize, Default)]
pub struct ReviveQ {
    tab: Option<String>,
    swapped: Option<String>,
}

fn fmt_date(lang: Lang, t: chrono::DateTime<chrono::Utc>) -> String {
    match lang {
        Lang::Tr => t.format("%d.%m.%Y").to_string(),
        Lang::En => t.format("%b %-d, %Y").to_string(),
    }
}

pub async fn revive_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    q: Option<Query<ReviveQ>>,
) -> Response {
    let mut c = ctx(&s, &headers, &uri, "quests").await;
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let q = q.map(|Query(q)| q).unwrap_or_default();
    let tab = if q.tab.as_deref() == Some("given") {
        "given"
    } else {
        "taken"
    };
    let mut tpl_revives = Vec::new();
    let mut tpl_given = Vec::new();
    let mut swapped = false;
    if let Some(pool) = &s.pool {
        let reported = reported_set(&s, Some(&me)).await;
        if tab == "taken" {
            match db::all_revives(pool, me.id).await {
                Ok(list) => {
                    for (i, r) in list.iter().enumerate() {
                        let mid = r.match_id.to_string();
                        if q.swapped.as_deref() == Some(mid.as_str()) {
                            swapped = true;
                        }
                        let mut card = CardView::new(&r.project, Some(&me), &reported, c.lang);
                        card.action = false;
                        let step3 = match r.project.skill_needed.first() {
                            Some(sk) => c.tf("revive.step3", sk),
                            None => c.t("revive.step3_plain").to_string(),
                        };
                        tpl_revives.push(ReviveView {
                            open: i == 0,
                            taken_on: c.tf("common.taken_on", fmt_date(c.lang, r.matched_at)),
                            match_id: mid,
                            card,
                            steps: vec![
                                c.t("revive.step1").to_string(),
                                c.t("revive.step2").to_string(),
                                step3,
                                c.t("revive.step4").to_string(),
                            ],
                        });
                    }
                }
                Err(_) => c.db_down = true,
            }
        } else {
            match db::my_quests(pool, me.id).await {
                Ok(list) => {
                    for qd in list {
                        let mut card = CardView::new(&qd.row, Some(&me), &reported, c.lang);
                        card.action = false;
                        let status_label = c.t(&format!("revive.status_{}", qd.status));
                        tpl_given.push(GivenView {
                            card,
                            status: qd.status,
                            status_label,
                        });
                    }
                }
                Err(_) => c.db_down = true,
            }
        }
    }
    render(
        &headers,
        StatusCode::OK,
        ReviveTpl {
            c,
            tab,
            revives: tpl_revives,
            given: tpl_given,
            swapped,
        },
    )
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "me.html")]
struct MeTpl {
    c: Ctx,
    handle: String,
    email: String,
    can_do: String,
    looking_for: String,
    saved: bool,
    error: Option<&'static str>,
    open: Vec<CardView>,
}

#[derive(Deserialize, Default)]
pub struct MeQ {
    saved: Option<String>,
}

async fn me_tpl(s: &AppState, c: Ctx, me: &Me) -> Result<MeTpl, Ctx> {
    let Some(pool) = &s.pool else { return Err(c) };
    let info = match db::user_info(pool, me.id).await {
        Ok(Some(u)) => u,
        _ => return Err(c),
    };
    let open = match db::my_quests(pool, me.id).await {
        Ok(list) => list
            .into_iter()
            .filter(|q| q.status == "OPEN")
            .map(|q| {
                let mut card = CardView::new(&q.row, Some(me), &HashSet::new(), c.lang);
                card.action = false;
                card
            })
            .collect(),
        Err(_) => return Err(c),
    };
    Ok(MeTpl {
        c,
        handle: info.handle,
        email: info.email,
        can_do: info.can_do.join(", "),
        looking_for: info.looking_for.join(", "),
        saved: false,
        error: None,
        open,
    })
}

pub async fn me_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    q: Option<Query<MeQ>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "me").await;
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    match me_tpl(&s, c, &me).await {
        Ok(mut tpl) => {
            tpl.saved = q.is_some_and(|Query(q)| q.saved.is_some());
            render(&headers, StatusCode::OK, tpl)
        }
        Err(c) => db_down_page(&headers, c),
    }
}

#[derive(Deserialize, Default)]
pub struct MeForm {
    #[serde(default)]
    can_do: String,
    #[serde(default)]
    looking_for: String,
}

pub async fn me_post(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    form: Option<Form<MeForm>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "me").await;
    if !same_origin(&headers) {
        return csrf_page(c, &headers);
    }
    let Some(me) = c.user.clone() else {
        return login_redirect(&headers, &c);
    };
    let f = form.map(|Form(f)| f).unwrap_or_default();
    let Some(pool) = s.pool.clone() else {
        return db_down_page(&headers, c);
    };
    let info = match db::user_info(&pool, me.id).await {
        Ok(Some(u)) => u,
        _ => return db_down_page(&headers, c),
    };
    let raw = validate::RawProfile {
        handle: info.handle,
        email: info.email,
        can_do: split_list(&f.can_do),
        looking_for: split_list(&f.looking_for),
    };
    match validate::validate_profile(&raw) {
        Ok(v) => match db::set_skills(&pool, me.id, &v.can_do, &v.looking_for).await {
            Ok(()) => redirect(&headers, "/me?saved=1"),
            Err(_) => db_down_page(&headers, c),
        },
        Err(_) => match me_tpl(&s, c, &me).await {
            Ok(mut tpl) => {
                tpl.can_do = f.can_do;
                tpl.looking_for = f.looking_for;
                tpl.error = Some(tpl.c.t("me.err_skill"));
                render(&headers, StatusCode::BAD_REQUEST, tpl)
            }
            Err(c) => db_down_page(&headers, c),
        },
    }
}

// ---------------------------------------------------------------------------
// Single quest (/q/{id})
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "quest.html")]
struct QuestTpl {
    c: Ctx,
    /// "open" | "moved"
    state: &'static str,
    card: Option<CardView>,
    dropped: bool,
    og_title: String,
    og_desc: String,
    og_url: String,
}

#[derive(Deserialize, Default)]
pub struct QuestQ {
    dropped: Option<String>,
}

pub async fn quest_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(id): Path<String>,
    q: Option<Query<QuestQ>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "pool").await;
    let Ok(id) = id.parse::<Uuid>() else {
        return not_found_page(&s, &headers, &uri).await;
    };
    let Some(pool) = &s.pool else {
        return db_down_page(&headers, c);
    };
    let p = match db::get_project(pool, id).await {
        Ok(Some(p)) if p.status != "CLOSED" => p,
        Ok(_) => return not_found_page(&s, &headers, &uri).await,
        Err(_) => return db_down_page(&headers, c),
    };
    let reported = reported_set(&s, c.user.as_ref()).await;
    let mut card = CardView::new(&p.row, c.user.as_ref(), &reported, c.lang);
    card.h = 2;
    let base = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let dropped = q.is_some_and(|Query(q)| q.dropped.is_some()) && card.own;
    let state = if p.status == "OPEN" { "open" } else { "moved" };
    if state == "moved" {
        card.action = false;
    }
    render(
        &headers,
        StatusCode::OK,
        QuestTpl {
            og_title: p.row.title.clone(),
            og_desc: p.row.one_liner.clone(),
            og_url: format!("{}/q/{}", base.trim_end_matches('/'), p.row.id),
            c,
            state,
            card: Some(card),
            dropped,
        },
    )
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "report_frag.html")]
struct ReportFragTpl {
    c: Ctx,
    id: String,
    msg: Option<&'static str>,
    error: Option<&'static str>,
    reason: String,
    hidden: bool,
}

#[derive(Deserialize, Default)]
pub struct ReportForm {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    back: String,
}

pub async fn report_post(
    State(s): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(id): Path<String>,
    form: Option<Form<ReportForm>>,
) -> Response {
    let c = ctx(&s, &headers, &uri, "").await;
    if !same_origin(&headers) {
        return csrf_page(c, &headers);
    }
    let f = form.map(|Form(f)| f).unwrap_or_default();
    let back = safe_next(Some(&f.back)).unwrap_or_else(|| "/".into());
    let Some(me) = c.user.clone() else {
        return redirect(&headers, &format!("/login?next={}", enc(&back)));
    };
    let Ok(pid) = id.parse::<Uuid>() else {
        return not_found_page(&s, &headers, &uri).await;
    };
    let Some(pool) = s.pool.clone() else {
        return db_down_page(&headers, c);
    };
    let raw = validate::RawReport {
        reporter_id: Some(me.id),
        project_id: Some(pid),
        reason: f.reason.clone(),
    };
    let (status, msg, error, hidden) = match validate::validate_report(&raw) {
        Err(_) => (
            StatusCode::BAD_REQUEST,
            None,
            Some("report.err_reason"),
            false,
        ),
        Ok(v) => match db::create_report(&pool, me.id, v.project_id, &v.reason).await {
            Ok((_, true)) => (StatusCode::CREATED, Some("report.hidden"), None, true),
            Ok((_, false)) => (StatusCode::CREATED, Some("report.thanks"), None, false),
            Err(DbError::Conflict(d)) if d.contains("already") => {
                (StatusCode::CONFLICT, Some("report.already"), None, false)
            }
            Err(DbError::Conflict(_)) => (StatusCode::CONFLICT, Some("report.own"), None, false),
            Err(DbError::NotFound(_)) => return not_found_page(&s, &headers, &uri).await,
            Err(_) => return db_down_page(&headers, c),
        },
    };
    if !is_htmx(&headers) {
        // Plain browser: a full page with the outcome and a way back.
        if let Some(e) = error {
            return message(
                &headers,
                c,
                status,
                "report.action",
                e,
                &back,
                "common.retry",
            );
        }
        let m = msg.unwrap_or("report.thanks");
        return message(
            &headers,
            c,
            status,
            "report.action",
            m,
            &back,
            "common.back_to_pool",
        );
    }
    render(
        &headers,
        status,
        ReportFragTpl {
            msg: msg.map(|k| c.t(k)),
            error: error.map(|k| c.t(k)),
            c,
            id: pid.to_string(),
            reason: f.reason,
            hidden,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_is_site_relative_only() {
        assert_eq!(safe_next(Some("/match")), Some("/match".into()));
        assert_eq!(safe_next(Some("/take/x?y=1")), Some("/take/x?y=1".into()));
        for bad in [
            "//evil.com",
            "https://evil.com",
            "/\\evil.com",
            "evil",
            "",
            "/a\nb",
        ] {
            assert_eq!(safe_next(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(safe_next(None), None);
    }

    #[test]
    fn same_origin_rules() {
        let mut h = HeaderMap::new();
        assert!(same_origin(&h), "no hints: not a cross-site browser form");
        h.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!same_origin(&h));
        h.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(same_origin(&h));
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("localhost:3000"));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:3000"),
        );
        assert!(same_origin(&h));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(!same_origin(&h));
        h.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(!same_origin(&h));
    }

    #[test]
    fn list_splitting() {
        assert_eq!(split_list(" rust, ,go ,"), vec!["rust", "go"]);
    }
}
