//! Matcher contract — frozen. Rule-based now, EmbeddingMatcher later.
//! Score = 80% fit + 20% random. Returns top-3 + 1 surprise.
//! Port of the original TypeScript RuleBasedMatcher.

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolItem {
    pub id: String,
    pub skill_needed: Vec<String>,
    pub time_bucket: String,
    pub energy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestUser {
    pub id: String,
    pub can_do: Vec<String>,
    pub looking_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let set_b: std::collections::HashSet<String> =
        b.iter().map(|s| s.to_lowercase()).collect();
    let hits = a
        .iter()
        .filter(|s| set_b.contains(&s.to_lowercase()))
        .count();
    hits as f64 / a.len().max(b.len()) as f64
}

impl Matcher for RuleMatcher {
    fn match_items(&self, user: &QuestUser, pool: &[PoolItem]) -> Vec<RankedItem> {
        let mut rng = rand::thread_rng();
        let mut scored: Vec<RankedItem> = pool
            .iter()
            .filter(|p| p.id != user.id)
            .map(|p| {
                let fit =
                    (overlap(&user.can_do, &p.skill_needed) + overlap(&user.looking_for, &p.skill_needed)) / 2.0;
                let rand_v: f64 = rng.gen();
                let score = (fit * 0.8 + rand_v * 0.2 * 100.0).round() / 100.0;
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
        scored.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap());
        let top3: Vec<RankedItem> = scored.iter().take(3).cloned().collect();
        let rest = &scored[top3.len().min(scored.len())..];
        let surprise: Vec<RankedItem> = rest.choose(&mut rng).cloned().into_iter().collect();
        [top3, surprise].concat()
    }
}

// Placeholder for phase C: same trait, pgvector-backed.
// pub struct EmbeddingMatcher;
// impl Matcher for EmbeddingMatcher { ... }

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
        let mut sorted = out.clone();
        sorted.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap());
        // top-3 ordered, surprise last — scores of first three non-increasing
        if out.len() >= 3 {
            assert!(out[0].score >= out[1].score && out[1].score >= out[2].score);
        }
    }
}
