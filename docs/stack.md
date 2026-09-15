> 🇹🇷 Türkçe için tıklayın: [stack.tr.md](./stack.tr.md)

# Stack — Locked (Rust)

Date: 2026-09-15

## Decision

- Web + API: Axum (Rust), REST under `/v1/`. Server-rendered Askama + HTMX, no Node. One repo, one binary. Bot reuses the same endpoints later.
- DB: Postgres + sqlx. Row-lock swap transaction + outbox table. `embedding` is a JSONB placeholder for now; real pgvector column arrives with phase C.
- Auth: email magic link IMPLEMENTED (15-min single-use tokens, HMAC sessions, LogMailer default / SMTP via SMTP_URL). Swaps, reports and profile edits require login; drops stay open intake.
- Voice storage: local `./data/voice` in MVP, served static as `voice_url`. No S3 setup. Moves to S3-compatible later without API change.
- Deploy: Docker single binary + Fly.io (any Docker host works).
- API contract: [openapi.yaml](./openapi.yaml) (hand-written, mirrors the code), RFC 9457 errors, `Idempotency-Key` on swap, cursor pagination.

## Why

- Modular monolith stays intact: B/C plug into the same REST + outbox, no core change.
- sqlx transaction covers atomic swap (two offers -> matched) with `SELECT ... FOR UPDATE`.
- pgvector arrives with phase C; until then the `Matcher` trait stays fixed for the future `EmbeddingMatcher` impl.

## Non-goals

- No microservices, no GraphQL, no S3 setup in MVP.

## Supply chain (audit round 10)

- Full-lockfile OSV scan: 1 hit, `rsa 0.9.10` (RUSTSEC-2023-0071, Marvin timing oracle).
- Verdict: NOT reachable. It arrives via `sqlx-macros-core` (host/build-time only);
  the production target graph excludes it (`cargo tree`: 0 matches) and the
  binary contains no `rsa`-crate symbols (only `ring`/`rustls` verification).
- `sqlx` uses `default-features = false` so no MySQL/SQLite drivers ship.
- Re-scan on every dependency change.
