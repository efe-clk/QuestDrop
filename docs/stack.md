> 🇹🇷 Türkçe için tıklayın: [stack.tr.md](./stack.tr.md)

# Stack — Locked (Rust)

Date: 2026-09-15

## Decision

- Web + API: Axum (Rust), REST under `/v1/`. Server-rendered Askama + HTMX, no Node. One repo, one binary. Bot reuses the same endpoints later.
- DB: Postgres + sqlx. Row-lock swap transaction + outbox table. `embedding` column reserved via pgvector, NULL for now.
- Auth: email magic link (lettre), signed session cookie. No passwords.
- Voice storage: local `./data/voice` in MVP, served static as `voice_url`. No S3 setup. Moves to S3-compatible later without API change.
- Deploy: Docker single binary + Fly.io (any Docker host works).
- API contract: OpenAPI 3.x, RFC 9457 errors, `Idempotency-Key` on swap, cursor pagination.

## Why

- Modular monolith stays intact: B/C plug into the same REST + outbox, no core change.
- sqlx transaction covers atomic swap (two offers -> matched) with `SELECT ... FOR UPDATE`.
- pgvector ready means C needs no migration, only a new `EmbeddingMatcher: Matcher` impl.

## Non-goals

- No microservices, no GraphQL, no S3 setup in MVP.
