//! Input validation for the Drop form (spec §5.1).
//! All rules are pure functions so they are unit-testable without a DB.

use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct RawDrop {
    pub handle: String,
    pub email: String,
    pub title: String,
    pub one_liner: String,
    pub link_url: String,
    pub voice_url: String,
    pub skill_needed: Vec<String>,
    pub time_bucket: String,
    pub energy: String,
}

#[derive(Debug)]
pub struct ValidatedDrop {
    pub handle: String,
    pub email: String,
    pub title: String,
    pub one_liner: String,
    pub link_url: String,
    pub voice_url: String,
    pub skill_needed: Vec<String>,
    /// Normalized to S | M | L
    pub time_bucket: String,
    /// Normalized to LOW | MID | HIGH
    pub energy: String,
}

fn valid_handle(h: &str) -> bool {
    (3..=24).contains(&h.len())
        && h.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn valid_email(e: &str) -> bool {
    if e.len() > 254 || e.len() < 5 {
        return false;
    }
    let mut parts = e.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !e.contains(' ')
        }
        _ => false,
    }
}

fn valid_http_url(u: &str, max_len: usize) -> bool {
    if u.len() > max_len || u.is_empty() {
        return false;
    }
    match url::Url::parse(u) {
        Ok(p) => p.scheme() == "http" || p.scheme() == "https",
        Err(_) => false,
    }
}

/// Voice must be an mp3: either a /voice/ relative path (local MVP storage)
/// or an http(s) URL whose PATH ends in .mp3 (query strings ignored, so
/// future signed URLs keep working). Duration (10-60s) is checked at upload
/// time in a later phase; the format gate lives here.
fn valid_voice(v: &str) -> bool {
    if v.len() > 2048 || v.is_empty() {
        return false;
    }
    if v.starts_with("/voice/") && !v.contains("..") {
        return v.to_lowercase().ends_with(".mp3");
    }
    match url::Url::parse(v) {
        Ok(p) => {
            (p.scheme() == "http" || p.scheme() == "https")
                && p.path().to_lowercase().ends_with(".mp3")
        }
        Err(_) => false,
    }
}

pub fn validate_drop(r: &RawDrop) -> Result<ValidatedDrop, Vec<String>> {
    let mut errs: Vec<String> = Vec::new();

    let handle = r.handle.trim().to_string();
    if !valid_handle(&handle) {
        errs.push("handle must be 3-24 chars: lowercase letters, digits, underscore".into());
    }
    let email = r.email.trim().to_lowercase();
    if !valid_email(&email) {
        errs.push("email is invalid".into());
    }
    let title = r.title.trim().to_string();
    if !(3..=80).contains(&title.chars().count()) {
        errs.push("title must be 3-80 chars".into());
    }
    let one_liner = r.one_liner.trim().to_string();
    if !(10..=200).contains(&one_liner.chars().count()) {
        errs.push("one_liner must be 10-200 chars".into());
    }
    let link_url = r.link_url.trim().to_string();
    if !valid_http_url(&link_url, 2048) {
        errs.push("link_url must be an http(s) URL".into());
    }
    let voice_url = r.voice_url.trim().to_string();
    if !valid_voice(&voice_url) {
        errs.push("voice_url must be an mp3 (/voice/... path or http(s) URL)".into());
    }
    let tb = r.time_bucket.trim().to_uppercase();
    if !matches!(tb.as_str(), "S" | "M" | "L") {
        errs.push("time_bucket must be S, M or L".into());
    }
    let en = r.energy.trim().to_uppercase();
    if !matches!(en.as_str(), "LOW" | "MID" | "HIGH") {
        errs.push("energy must be LOW, MID or HIGH".into());
    }
    let mut skills: Vec<String> = r
        .skill_needed
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    skills.sort();
    skills.dedup();
    skills.truncate(10);
    if skills.iter().any(|s| s.chars().count() > 32) {
        errs.push("each skill must be 1-32 chars".into());
    }

    if errs.is_empty() {
        Ok(ValidatedDrop {
            handle,
            email,
            title,
            one_liner,
            link_url,
            voice_url,
            skill_needed: skills,
            time_bucket: tb,
            energy: en,
        })
    } else {
        Err(errs)
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct RawProfile {
    pub handle: String,
    pub email: String,
    pub can_do: Vec<String>,
    pub looking_for: Vec<String>,
}

#[derive(Debug)]
pub struct ValidatedProfile {
    pub handle: String,
    pub email: String,
    pub can_do: Vec<String>,
    pub looking_for: Vec<String>,
}

fn clean_skills(raw: &[String], errs: &mut Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = raw
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out.truncate(20);
    if out.iter().any(|s| s.chars().count() > 32) {
        errs.push("each skill must be 1-32 chars".into());
    }
    out
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct RawReport {
    pub reporter_id: Option<uuid::Uuid>,
    pub project_id: Option<uuid::Uuid>,
    pub reason: String,
}

#[derive(Debug)]
pub struct ValidatedReport {
    pub reporter_id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub reason: String,
}

pub fn validate_report(r: &RawReport) -> Result<ValidatedReport, Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    let reporter_id = match r.reporter_id {
        Some(id) => id,
        None => {
            errs.push("reporter_id is required".into());
            uuid::Uuid::nil()
        }
    };
    let project_id = match r.project_id {
        Some(id) => id,
        None => {
            errs.push("project_id is required".into());
            uuid::Uuid::nil()
        }
    };
    let reason = r.reason.trim().to_string();
    if !(10..=300).contains(&reason.chars().count()) {
        errs.push("reason must be 10-300 chars".into());
    }
    if errs.is_empty() {
        Ok(ValidatedReport {
            reporter_id,
            project_id,
            reason,
        })
    } else {
        Err(errs)
    }
}
/// Skill profile for matching. Without this every user has empty skills and
/// the matcher degrades to pure random — this endpoint keeps fit meaningful.
pub fn validate_profile(r: &RawProfile) -> Result<ValidatedProfile, Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    let handle = r.handle.trim().to_string();
    if !valid_handle(&handle) {
        errs.push("handle must be 3-24 chars: lowercase letters, digits, underscore".into());
    }
    let email = r.email.trim().to_lowercase();
    if !valid_email(&email) {
        errs.push("email is invalid".into());
    }
    let can_do = clean_skills(&r.can_do, &mut errs);
    let looking_for = clean_skills(&r.looking_for, &mut errs);
    if errs.is_empty() {
        Ok(ValidatedProfile {
            handle,
            email,
            can_do,
            looking_for,
        })
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> RawDrop {
        RawDrop {
            handle: "rust_dev".into(),
            email: "dev@example.com".into(),
            title: "Half-finished CLI".into(),
            one_liner: "A CLI that renames photos but the config part is missing".into(),
            link_url: "https://github.com/x/y".into(),
            voice_url: "/voice/abc.mp3".into(),
            skill_needed: vec!["Rust".into()],
            time_bucket: "s".into(),
            energy: "low".into(),
        }
    }

    #[test]
    fn accepts_good_and_normalizes() {
        let v = validate_drop(&good()).expect("good input must pass");
        assert_eq!(v.time_bucket, "S");
        assert_eq!(v.energy, "LOW");
        assert_eq!(v.skill_needed, vec!["rust"]);
    }

    #[test]
    fn email_is_lowercased() {
        // Same mailbox in different cases must map to one identity,
        // otherwise the daily cap can be bypassed with case variants.
        let mut mixed = good();
        mixed.email = "User@X.COM".into();
        let v = validate_drop(&mixed).expect("must pass");
        assert_eq!(v.email, "user@x.com");
    }

    #[test]
    fn collects_all_errors() {
        let mut bad = good();
        bad.handle = "AB".into();
        bad.email = "not-an-email".into();
        bad.title = "x".into();
        bad.link_url = "ftp://evil".into();
        bad.voice_url = "".into();
        bad.time_bucket = "XXL".into();
        bad.energy = "max".into();
        let errs = validate_drop(&bad).expect_err("bad input must fail");
        assert!(errs.len() >= 7, "expected all errors, got: {errs:?}");
    }

    #[test]
    fn rejects_js_scheme_and_traversal_voice() {
        let mut bad = good();
        bad.link_url = "javascript:alert(1)".into();
        assert!(validate_drop(&bad).is_err());
        bad.link_url = "https://github.com/x/y".into();
        bad.voice_url = "/voice/../../etc/passwd.mp3".into();
        assert!(validate_drop(&bad).is_err());
        bad.voice_url = "/VOICE/abc.mp3".into();
        assert!(
            validate_drop(&bad).is_err(),
            "case-variant prefix 404s on Linux"
        );
    }

    #[test]
    fn voice_accepts_signed_urls_rejects_query_tricks() {
        let mut ok = good();
        // Future S3-style signed URL: suffix lives in the path, not the query.
        ok.voice_url = "https://cdn.x.com/a.mp3?sig=abc123&exp=99".into();
        assert!(validate_drop(&ok).is_ok());
        // .mp3 only in the query string is not an mp3 path.
        ok.voice_url = "https://x.com/y?f=.mp3".into();
        assert!(validate_drop(&ok).is_err());
        // Uppercase extension on remote URLs is fine.
        ok.voice_url = "https://x.com/a.MP3".into();
        assert!(validate_drop(&ok).is_ok());
    }

    #[test]
    fn counts_chars_not_bytes_and_dedupes_skills() {
        let mut ok = good();
        ok.title = "ç".repeat(80);
        ok.skill_needed = vec!["Rust".into(), "rust".into(), " RUST ".into()];
        let v = validate_drop(&ok).expect("80 chars must pass regardless of bytes");
        assert_eq!(v.skill_needed, vec!["rust"]);
        ok.title = "ç".repeat(81);
        assert!(validate_drop(&ok).is_err(), "81 chars must fail");
    }
}
