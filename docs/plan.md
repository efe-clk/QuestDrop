> 🇹🇷 Türkçe için tıklayın: [plan.tr.md](./plan.tr.md)

# QuestDrop — Rust Build Plan

Status: Phase 0 done, Phase 1 done (2026-09-15, 12 tests green + live E2E).

Stack: Axum + sqlx + Postgres + Askama/HTMX. Single binary, `/v1/` REST.

## Phase 0 — Scaffold (4h)

1. `cargo init`, Axum router + `/health`, Askama base layout
2. sqlx + Postgres connect, `sqlx migrate` setup
3. Dockerfile + Fly.io config, `./data/voice` static serve

Done: `cargo run` serves home + health.

## Phase 1 — Drop + Freeze (1 day)

1. Tables: users, projects, swap_offers, outbox
2. `POST /v1/projects`: 30s form, link + voice required, time_bucket + energy validation
3. Freeze: insert project (open) + swap_offer (open) in one transaction

Done: drop form saves and lists in pool.

## Phase 2 — Match (1 day)

1. `trait Matcher`, `RuleMatcher`: 80% fit (skills + time + energy) + 20% random
2. `GET /v1/match?user=`: top-3 + 1 surprise
3. Unit test: matcher score ordering

Done: pool returns ranked 4.

## Phase 3 — Swap atomic (1 day)

1. `POST /v1/swaps` with `Idempotency-Key`, `SELECT ... FOR UPDATE`
2. Single transaction: two offers -> matched, insert match row, write outbox
3. Integration test: double-claim blocked

Done: two users swap without race.

## Phase 4 — Revive + Moderation (half day)

1. Revive view: freeze package + first 2-minute task
2. Rules: max 3 drops/day, no self-match, 3 reports = auto-hide
3. RFC 9457 errors, cursor pagination on pool

Done: E2E drop -> match -> take passes.

## B/C ports (later)

- B: bot calls same `/v1/`, listens to outbox.
- C: `EmbeddingMatcher: Matcher`, pgvector column fill.
