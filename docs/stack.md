> 🇹🇷 Türkçe için tıklayın: [stack.tr.md](./stack.tr.md)

# Stack — Locked (Rust)

Date: 2026-09-15 · Last checked against the code: 2026-10-12

## Decision

- Web + API: Axum 0.7 (Rust), REST under `/v1/`. Server-rendered Askama + HTMX, no Node. One repo, one binary. Pages live in `templates/` (compiled in), strings in `i18n/{en,tr}.toml` (compiled in), and `static/` holds one CSS file, two small JS files and a pinned HTMX 2.0.10 (served from disk, so the image copies `static/`); see [web-ui.md](./web-ui.md).
- DB: Postgres 13+ (uses built-in `gen_random_uuid()`) + sqlx 0.8 with runtime queries only, so building needs no database. Migrations in `migrations/` run on boot. Row-lock swap transaction + outbox table. `embedding` is an unused JSONB placeholder; a pgvector column can replace it later. Phase C v1 does not use it.
- Auth: email magic link (15-min single-use tokens, HMAC-signed 30-day sessions, LogMailer by default, SMTP via `SMTP_URL`). Drops, matches, revives, swaps, reports, profile edits and voice uploads need login.
- Voice storage: local `VOICE_DIR` (default `./data/voice`), served at `/voice/*`. mp3, ogg, webm or m4a (container sniffed from magic bytes), 5 MB per file, `VOICE_QUOTA_MB` total (default 500). No S3. It can move to S3-compatible storage later without an API change.
- Deploy: Docker image with the `questdrop` binary (digest-pinned `rust:1.90-slim` build → `debian:bookworm-slim`, non-root user). `fly.toml` targets Fly.io (`fra`, `/ready` check), but any Docker host works. **Not deployed yet.** The `bot` binary is not in the image.
- API contract: [openapi.yaml](./openapi.yaml) (hand-written, mirrors the code), RFC 9457 errors, `Idempotency-Key` on swap, cursor pagination.
- Defaults: 10 s request timeout, 2 MB body limit (Axum default, lifted only on `/v1/voice`), `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, 5 s Postgres statement timeout, pool of 5 connections.

## Why

- Modular monolith stays intact: B and C plugged into the same REST + outbox without core changes.
- An sqlx transaction covers the atomic swap (two offers → MATCHED) with `SELECT ... FOR UPDATE`.
- The `Matcher` trait stays fixed, so a future pgvector-backed matcher can replace the local `EmbeddingMatcher` without touching callers.

## Non-goals

- No microservices, no GraphQL, no S3 setup in MVP.

## Supply chain (audit round 10)

- Full-lockfile OSV scan: 1 hit, `rsa 0.9.10` (RUSTSEC-2023-0071, Marvin timing oracle).
- Verdict: NOT reachable. It arrives via `sqlx-macros-core` (host/build-time only);
  the production target graph excludes it (`cargo tree`: 0 matches) and the
  binary contains no `rsa`-crate symbols (only `ring`/`rustls` verification).
- `sqlx` uses `default-features = false` so no MySQL/SQLite drivers ship.
- Re-scan on every dependency change.
