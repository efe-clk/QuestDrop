//! Matcher contract — frozen. Rule-based now, EmbeddingMatcher later.
//! Score = 80% fit + 20% random. Returns top-3 + 1 surprise.
//! Port of the original TypeScript RuleBasedMatcher.

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PoolItem {
    pub id: String,
    pub skill_needed: Vec<String>,
    pub time_bucket: String,
    pub energy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct QuestUser {
    pub id: String,
    pub can_do: Vec<String>,
    pub looking_for: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedItem {
    pub id: String,
    pub skill_needed: Vec<String>,
    pub time_bucket: String,
    pub energy: String,
    pub score: f64,
    pub reason: String,
}

pub trait Matcher {
    fn match_items(&self, user: &QuestUser, pool: &[PoolItem]) -> Vec<RankedItem>;
}

pub struct RuleMatcher;

fn overlap(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_b: std::collections::HashSet<String> = b.iter().map(|s| s.to_lowercase()).collect();
    let hits = a
        .iter()
        .filter(|s| set_b.contains(&s.to_lowercase()))
        .count();
    hits as f64 / a.len().max(b.len()) as f64
}

/// Deterministic skill fit in 0..=1 (the 80% part of the score, no random).
/// Used for swap records so a match keeps an explainable score.
pub fn fit_score(user: &QuestUser, item: &PoolItem) -> f64 {
    let fit = (overlap(&user.can_do, &item.skill_needed)
        + overlap(&user.looking_for, &item.skill_needed))
        / 2.0;
    (fit * 100.0).round() / 100.0
}

impl Matcher for RuleMatcher {
    fn match_items(&self, user: &QuestUser, pool: &[PoolItem]) -> Vec<RankedItem> {
        let mut rng = rand::thread_rng();
        let mut scored: Vec<RankedItem> = pool
            .iter()
            .filter(|p| p.id != user.id)
            .map(|p| {
                let fit = fit_score(user, p);
                let rand_v: f64 = rng.gen();
                let score = ((fit * 0.8 + rand_v * 0.2) * 100.0).round() / 100.0;
                RankedItem {
                    id: p.id.clone(),
                    skill_needed: p.skill_needed.clone(),
                    time_bucket: p.time_bucket.clone(),
                    energy: p.energy.clone(),
                    score,
                    reason: format!("fit={fit:.2} rand={rand_v:.2}"),
                }
            })
            .collect();
        scored.sort_by(|x, y| y.score.total_cmp(&x.score));
        let top3: Vec<RankedItem> = scored.iter().take(3).cloned().collect();
        let rest = &scored[top3.len().min(scored.len())..];
        let surprise: Vec<RankedItem> = rest.choose(&mut rng).cloned().into_iter().collect();
        [top3, surprise].concat()
    }
}

/// C phase: deterministic local text embedding behind the same trait.
/// No external model or API key needed: char-trigram hashing into a fixed
/// vector + cosine similarity. Fully deterministic (no random term), so the
/// same pool always ranks the same. A future pgvector-backed matcher swaps
/// in without touching callers.
pub struct EmbeddingMatcher {
    dim: usize,
}

impl Default for EmbeddingMatcher {
    fn default() -> Self {
        Self { dim: 256 }
    }
}

impl EmbeddingMatcher {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(16) }
    }

    fn user_text(user: &QuestUser) -> String {
        format!("{} {}", user.can_do.join(" "), user.looking_for.join(" ")).to_lowercase()
    }

    fn item_text(item: &PoolItem) -> String {
        item.skill_needed.join(" ").to_lowercase()
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return v;
        }
        // Trigrams with padding so short texts still fill buckets.
        let mut grams: Vec<String> = Vec::new();
        if chars.len() < 3 {
            grams.push(chars.iter().collect());
        } else {
            for w in chars.windows(3) {
                grams.push(w.iter().collect());
            }
        }
        for g in grams {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in g.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            v[(h as usize) % self.dim] += 1.0;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }

    fn cosine(a: &[f32], b: &[f32]) -> f64 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        ((dot * 100.0).round() / 100.0) as f64
    }
}

impl Matcher for EmbeddingMatcher {
    fn match_items(&self, user: &QuestUser, pool: &[PoolItem]) -> Vec<RankedItem> {
        let uvec = self.embed(&Self::user_text(user));
        let mut scored: Vec<RankedItem> = pool
            .iter()
            .filter(|p| p.id != user.id)
            .map(|p| {
                let cos = Self::cosine(&uvec, &self.embed(&Self::item_text(p)));
                RankedItem {
                    id: p.id.clone(),
                    skill_needed: p.skill_needed.clone(),
                    time_bucket: p.time_bucket.clone(),
                    energy: p.energy.clone(),
                    score: cos,
                    reason: format!("cos={cos:.2}"),
                }
            })
            .collect();
        scored.sort_by(|x, y| y.score.total_cmp(&x.score));
        scored.truncate(4);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, skills: &[&str]) -> PoolItem {
        PoolItem {
            id: id.into(),
            skill_needed: skills.iter().map(|s| s.to_string()).collect(),
            time_bucket: "S".into(),
            energy: "LOW".into(),
        }
    }

    #[test]
    fn excludes_self_and_orders() {
        let m = RuleMatcher;
        let user = QuestUser {
            id: "u1".into(),
            can_do: vec!["rust".into()],
            looking_for: vec![],
        };
        let pool = vec![
            item("u1", &["rust"]),
            item("p1", &["rust"]),
            item("p2", &["go"]),
            item("p3", &["rust"]),
            item("p4", &["python"]),
            item("p5", &["rust"]),
        ];
        let out = m.match_items(&user, &pool);
        assert!(out.iter().all(|r| r.id != "u1"));
        assert!(out.len() <= 4);
        // top-3 ordered, surprise last — scores of first three non-increasing
        if out.len() >= 3 {
            assert!(out[0].score >= out[1].score && out[1].score >= out[2].score);
        }
    }

    #[test]
    fn scores_stay_in_range() {
        // Regression test: operator-precedence bug once pushed scores above 1.0
        // because the random term was scaled by 100 before rounding.
        let m = RuleMatcher;
        let user = QuestUser {
            id: "u1".into(),
            can_do: vec!["rust".into()],
            looking_for: vec!["go".into()],
        };
        let pool = vec![item("p1", &["rust"]), item("p2", &["go"])];
        for _ in 0..200 {
            for r in m.match_items(&user, &pool) {
                assert!(
                    (0.0..=1.0).contains(&r.score),
                    "score out of range: {}",
                    r.score
                );
            }
        }
    }

    #[test]
    fn embedding_ranks_skill_overlap_first_deterministically() {
        use super::{EmbeddingMatcher, Matcher};
        let m = EmbeddingMatcher::default();
        let user = QuestUser {
            id: "u1".into(),
            can_do: vec!["rust".into(), "axum".into()],
            looking_for: vec![],
        };
        let pool = vec![
            item("p1", &["python", "django"]),
            item("p2", &["rust", "axum"]),
            item("p3", &["go", "gin"]),
        ];
        let a = m.match_items(&user, &pool);
        let b = m.match_items(&user, &pool);
        assert_eq!(a[0].id, "p2", "skill overlap must rank first");
        assert_eq!(a, b, "embedding matcher must be deterministic");
        assert!(a.iter().all(|r| (0.0..=1.0).contains(&r.score)));
    }
}
