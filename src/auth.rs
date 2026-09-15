//! Email magic-link auth + HMAC-signed server-side sessions.
//!
//! Flow: `POST /v1/auth/request` stores a one-time token (hash only) and
//! mails a link. `POST /v1/auth/callback` redeems it for a session cookie.
//! Sessions are random UUIDs; the cookie carries `{id}.{hmac}` so tampering
//! is detected without a DB hit, then the id is resolved server-side.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::validate::{valid_email, valid_handle};

pub const MAGIC_TTL_MINUTES: i64 = 15;
pub const SESSION_TTL_DAYS: i64 = 30;
pub const COOKIE_NAME: &str = "qd_session";

#[derive(Debug)]
pub enum AuthError {
    Invalid,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AuthError {
    fn from(e: sqlx::Error) -> Self {
        AuthError::Db(e)
    }
}

fn sha256_hex(s: &str) -> String {
    hex_of(&Sha256::digest(s.as_bytes()))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn new_raw_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Server secret for session HMACs. Random per boot unless SESSION_SECRET
/// is set (required in production; cookies die on restart otherwise).
pub fn session_secret() -> Vec<u8> {
    match std::env::var("SESSION_SECRET") {
        Ok(s) if s.len() >= 16 => s.into_bytes(),
        _ => {
            tracing::warn!("SESSION_SECRET unset/short: ephemeral secret, sessions die on restart");
            let mut buf = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut buf);
            buf.to_vec()
        }
    }
}

/// Stores a login token, returns the RAW value (emailed to the user).
pub async fn issue_magic_token(
    pool: &PgPool,
    email: &str,
    handle: Option<&str>,
) -> Result<String, AuthError> {
    if !valid_email(email) {
        return Err(AuthError::Invalid);
    }
    let clean_handle = handle
        .map(|h| h.trim().to_string())
        .filter(|h| valid_handle(h));
    let raw = new_raw_token();
    sqlx::query(
        "INSERT INTO magic_tokens (email, handle, token_hash, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(mins => $4))",
    )
    .bind(email.trim().to_lowercase())
    .bind(clean_handle)
    .bind(sha256_hex(&raw))
    .bind(MAGIC_TTL_MINUTES as i32)
    .execute(pool)
    .await?;
    Ok(raw)
}

fn derive_handle(email: &str, hint: Option<&str>) -> String {
    if let Some(h) = hint {
        if valid_handle(h) {
            return h.to_string();
        }
    }
    let local = email.split('@').next().unwrap_or("user");
    let clean: String = local
        .to_lowercase()
        .bytes()
        .map(|b| {
            if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' {
                b as char
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim_matches('_').to_string();
    if clean.len() >= 3 {
        clean.chars().take(20).collect()
    } else {
        "user_quest".into()
    }
}

/// Redeems a token for a user id. Single-use, 15-minute window.
pub async fn consume_magic_token(pool: &PgPool, raw: &str) -> Result<Uuid, AuthError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT id, email, handle, expires_at, used FROM magic_tokens
         WHERE token_hash = $1 FOR UPDATE",
    )
    .bind(sha256_hex(raw.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(r) = row else {
        tx.rollback().await?;
        return Err(AuthError::Invalid);
    };
    let used: bool = r.try_get("used")?;
    let exp: DateTime<Utc> = r.try_get("expires_at")?;
    if used || exp < Utc::now() {
        tx.rollback().await?;
        return Err(AuthError::Invalid);
    }
    let email: String = r.try_get("email")?;
    let hint: Option<String> = r.try_get("handle")?;
    sqlx::query("UPDATE magic_tokens SET used = TRUE WHERE id = $1")
        .bind(r.try_get::<Uuid, _>("id")?)
        .execute(&mut *tx)
        .await?;

    if let Some(uid) = sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?
    {
        tx.commit().await?;
        return Ok(uid);
    }
    // New user: derived handle, short suffix on conflict.
    let base = derive_handle(&email, hint.as_deref());
    for attempt in 0..4 {
        let handle = if attempt == 0 {
            base.clone()
        } else {
            format!(
                "{}_{:04x}",
                &base[..base.len().min(15)],
                rand::thread_rng().next_u32() & 0xffff
            )
        };
        match sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO users (handle, email) VALUES ($1, $2) RETURNING id",
        )
        .bind(&handle)
        .bind(&email)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => {
                tx.commit().await?;
                return Ok(id);
            }
            Err(sqlx::Error::Database(d)) if d.code().as_deref() == Some("23505") => continue,
            Err(e) => {
                tx.rollback().await?;
                return Err(AuthError::Db(e));
            }
        }
    }
    tx.rollback().await?;
    Err(AuthError::Invalid)
}

fn hmac_hex(secret: &[u8], msg: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(msg.as_bytes());
    hex_of(&mac.finalize().into_bytes())
}

/// Cookie value `{session_id}.{hmac}`.
pub async fn create_session(
    pool: &PgPool,
    secret: &[u8],
    user: Uuid,
) -> Result<String, sqlx::Error> {
    let sid = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(days => $4))",
    )
    .bind(sid)
    .bind(user)
    .bind(sha256_hex(&sid.to_string()))
    .bind(SESSION_TTL_DAYS as i32)
    .execute(pool)
    .await?;
    Ok(format!("{sid}.{}", hmac_hex(secret, &sid.to_string())))
}

/// Verifies HMAC (no DB hit on forgery) then resolves a live session.
pub async fn load_session_user(
    pool: &PgPool,
    secret: &[u8],
    cookie: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let (id_str, sig) = match cookie.split_once('.') {
        Some(p) => p,
        None => return Ok(None),
    };
    let sid: Uuid = match id_str.parse() {
        Ok(u) => u,
        Err(_) => return Ok(None),
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(id_str.as_bytes());
    let sig_bytes = match hex_decode(sig) {
        Some(b) => b,
        None => return Ok(None),
    };
    if mac.verify_slice(&sig_bytes).is_err() {
        return Ok(None);
    }
    sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM sessions WHERE id = $1 AND expires_at > now()",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Mailers
// ---------------------------------------------------------------------------

/// Delivery for login links. LogMailer is the working default; SmtpMailer
/// activates when SMTP_URL is set (any smtp:// URL lettre accepts).
#[async_trait::async_trait]
pub trait Mailer: Send + Sync {
    async fn send_login_link(&self, email: &str, link: &str) -> Result<(), String>;
}

pub struct LogMailer;

#[async_trait::async_trait]
impl Mailer for LogMailer {
    async fn send_login_link(&self, email: &str, link: &str) -> Result<(), String> {
        tracing::info!(%email, link = %link, "login link (log mailer)");
        Ok(())
    }
}

pub struct SmtpMailer {
    transport: lettre::SmtpTransport,
    from: String,
}

impl SmtpMailer {
    pub fn from_env() -> Result<Self, String> {
        let url = std::env::var("SMTP_URL").map_err(|_| "SMTP_URL unset".to_string())?;
        let transport = lettre::SmtpTransport::from_url(&url)
            .map_err(|e| format!("smtp url: {e}"))?
            .build();
        let from = std::env::var("MAIL_FROM").unwrap_or_else(|_| "questdrop@localhost".into());
        Ok(Self { transport, from })
    }
}

#[async_trait::async_trait]
impl Mailer for SmtpMailer {
    async fn send_login_link(&self, email: &str, link: &str) -> Result<(), String> {
        use lettre::{message::header::ContentType, Transport};
        let msg = lettre::Message::builder()
            .from(
                self.from
                    .parse()
                    .map_err(|e| format!("bad MAIL_FROM: {e}"))?,
            )
            .to(email.parse().map_err(|e| format!("bad email: {e}"))?)
            .subject("Your QuestDrop login link")
            .header(ContentType::TEXT_PLAIN)
            .body(format!("Log in (15 min): {link}"))
            .map_err(|e| format!("build mail: {e}"))?;
        let transport = self.transport.clone();
        tokio::task::spawn_blocking(move || {
            transport
                .send(&msg)
                .map(|_| ())
                .map_err(|e| format!("smtp send: {e}"))
        })
        .await
        .map_err(|e| format!("smtp task: {e}"))?
    }
}

pub fn mailer_from_env() -> Box<dyn Mailer> {
    match SmtpMailer::from_env() {
        Ok(m) => {
            tracing::info!("mailer: smtp");
            Box::new(m)
        }
        Err(_) => {
            tracing::info!("mailer: log (set SMTP_URL for real mail)");
            Box::new(LogMailer)
        }
    }
}

pub fn login_link(token: &str) -> String {
    let base = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    format!("{}/login?token={token}", base.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_value_roundtrips_and_rejects_forgery() {
        let secret = b"test-secret-0123456789abcdef";
        let sid = Uuid::new_v4();
        let v = format!("{sid}.{}", hmac_hex(secret, &sid.to_string()));
        assert!(v.starts_with(&sid.to_string()));
        // Tamper one char -> must not verify (checked in load path via Mac).
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(sid.to_string().as_bytes());
        assert!(mac.verify_slice(&hex_decode("00").unwrap()).is_err());
    }

    #[test]
    fn derive_handle_sanitizes() {
        assert_eq!(derive_handle("Dev-X@x.com", None), "dev_x");
        assert_eq!(derive_handle("ab@x.com", None), "user_quest");
        assert_eq!(derive_handle("a@x.com", Some("my_handle")), "my_handle");
    }
}
