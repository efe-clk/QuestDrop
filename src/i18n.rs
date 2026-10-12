//! UI dictionaries (web-ui §9): every user-facing string lives in
//! `i18n/{en,tr}.toml`, never in templates. Compiled into the binary and
//! flattened to `section.key` on first use. Missing keys fall back to
//! English, then to the key itself, so a typo never breaks a page.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Tr,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Tr => "tr",
        }
    }

    pub fn other(self) -> Lang {
        match self {
            Lang::En => Lang::Tr,
            Lang::Tr => Lang::En,
        }
    }

    pub fn from_code(s: &str) -> Option<Lang> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "tr" => Some(Lang::Tr),
            _ => None,
        }
    }

    /// First supported language in an Accept-Language header (order of
    /// appearance; q-values are rarely meaningful in practice).
    pub fn from_accept_language(header: &str) -> Option<Lang> {
        header.split(',').find_map(|part| {
            let tag = part.split(';').next()?.trim();
            let primary = tag.split('-').next()?;
            Lang::from_code(primary)
        })
    }
}

type Dict = HashMap<String, String>;

fn flatten(prefix: &str, v: &toml::Value, out: &mut Dict) {
    match v {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        other => {
            out.insert(prefix.to_string(), other.to_string());
        }
    }
}

fn parse(src: &str) -> Dict {
    let mut out = Dict::new();
    match src.parse::<toml::Value>() {
        Ok(v) => flatten("", &v, &mut out),
        Err(e) => tracing::error!(error = %e, "i18n dictionary failed to parse"),
    }
    out
}

fn dicts() -> &'static (Dict, Dict) {
    static D: OnceLock<(Dict, Dict)> = OnceLock::new();
    D.get_or_init(|| {
        (
            parse(include_str!("../i18n/en.toml")),
            parse(include_str!("../i18n/tr.toml")),
        )
    })
}

/// Looks up `key` for `lang` (English fallback, then the key itself).
pub fn t(lang: Lang, key: &str) -> &'static str {
    let (en, tr) = dicts();
    let primary = match lang {
        Lang::En => en,
        Lang::Tr => tr,
    };
    if let Some(v) = primary.get(key).or_else(|| en.get(key)) {
        return v.as_str();
    }
    tracing::warn!(key, "missing i18n key");
    // Leak once per unknown key is acceptable: keys are compile-time
    // literals in templates, so the set is tiny and bounded.
    Box::leak(key.to_string().into_boxed_str())
}

/// `t` with `{x}` replaced by `arg`.
pub fn tf(lang: Lang, key: &str, arg: impl std::fmt::Display) -> String {
    t(lang, key).replace("{x}", &arg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_have_identical_keys() {
        let (en, tr) = dicts();
        assert!(en.len() > 100, "en dictionary parsed");
        let mut missing_tr: Vec<_> = en.keys().filter(|k| !tr.contains_key(*k)).collect();
        let mut missing_en: Vec<_> = tr.keys().filter(|k| !en.contains_key(*k)).collect();
        missing_tr.sort();
        missing_en.sort();
        assert!(missing_tr.is_empty(), "missing in tr.toml: {missing_tr:?}");
        assert!(missing_en.is_empty(), "missing in en.toml: {missing_en:?}");
    }

    #[test]
    fn lookup_and_fallbacks() {
        assert_eq!(t(Lang::Tr, "nav.pool"), "Havuz");
        assert_eq!(t(Lang::En, "nav.pool"), "Pool");
        assert_eq!(t(Lang::En, "no.such.key"), "no.such.key");
        assert_eq!(tf(Lang::En, "match.fit_pct", 50), "Overlaps 50% with your skills");
    }

    #[test]
    fn accept_language() {
        assert_eq!(Lang::from_accept_language("tr-TR,tr;q=0.9,en;q=0.8"), Some(Lang::Tr));
        assert_eq!(Lang::from_accept_language("de-DE,en-US;q=0.7"), Some(Lang::En));
        assert_eq!(Lang::from_accept_language("de"), None);
    }
}
