> 🇹🇷 Türkçe için tıklayın: [plan.tr.md](./plan.tr.md)

# QuestDrop — Rust Build Plan

Status (2026-10-12): **Backend and web UI done, not deployed.**
Phases 0-5 plus B and C are implemented: the API, and the browser pages on top of it (59 Rust tests + a browser E2E, all green).

Stack: Axum + sqlx + Postgres + Askama/HTMX. Single binary, `/v1/` REST.

## Phase 0 — Scaffold ✅

1. `cargo init`, Axum router + `/health`, Askama base layout
2. sqlx + Postgres connect, migrations run on boot
3. Dockerfile + Fly.io config, `/voice` static serve

Done: `cargo run` serves the home page + health.

## Phase 1 — Drop + Freeze ✅

1. Tables: users, projects, swap_offers, outbox_events
2. `POST /v1/projects`: link + voice required, time_bucket + energy validation
3. Freeze: project (OPEN) + offer (OPEN) + outbox event in one transaction

Done: the API saves drops and lists them in the pool; the web drop form (§ phase 5) uses it.

## Phase 2 — Match ✅

1. `trait Matcher`, `RuleMatcher`: 80% skill fit + 20% random (time/energy not scored yet)
2. `GET /v1/match?user_id=`: top-3 + 1 surprise
3. Unit test: score ordering and range

UI: `/match`.

## Phase 3 — Atomic swap ✅

1. `POST /v1/swaps` with `Idempotency-Key`, `SELECT ... FOR UPDATE` in id order
2. One transaction: two offers → MATCHED, match row, outbox event
3. Integration test: double-claim blocked, same-key race gives 1×201 + 4×200

UI: `/take/{offer_id}`.

## Phase 4 — Revive + Moderation ✅

1. Revive: freeze package + first 2-minute task
2. Rules: max 3 drops/day, no self-match, 3 reports = auto-hide
3. RFC 9457 errors, cursor pagination

UI: `/revive` and the report menu on every card.

## B / C ✅ (v1)

- B: outbox publisher + `bot` binary, log sink or Discord webhook sink.
- C: `EmbeddingMatcher` (local trigram + cosine) via `?algo=embedding`.

## Phase 5 — Web UI ✅

Server-rendered Askama + HTMX pages in the same binary (`src/web.rs`, `templates/`, `static/`), so the session cookie works without CORS. Contract: [web-ui.md](./web-ui.md). Every form works without JavaScript; HTMX (`hx-boost`) only speeds things up.

Pages: `/` pool (cursor "show more"), `/login` (+ `?token=` confirm button), `/drop` (browser recorder + file upload), `/match`, `/take/{offer_id}`, `/revive` (taken / dropped tabs, first 2 minutes), `/me`, `/q/{id}` (shareable, Open Graph), report menu on cards, logout. English + Turkish (`i18n/*.toml`).

API gaps closed for it (web-ui §10): `/login` page; `offer_id` on pool, match and project rows; `GET /v1/me/offers`; `GET /v1/projects/{id}`; voice accepts Opus (webm/ogg) and AAC (m4a) next to mp3; drops, `/v1/match` and `/v1/revive` require login; the dead `/v1/match` link is gone. CORS stays unneeded (same origin).

Tests: `tests/web.rs` (pages, login flow, CSRF, drop validation + cap + no-JS upload, match → take → revive, idempotent double submit, race state, report + auto-hide, pagination fragment, DB-down) and `e2e/` (Playwright: two users, fake-mic recording, real race, axe, 360/768/1280 light/dark screenshots).

Done when a stranger can drop, log in, match, swap and revive in a browser: ✅ (covered by the E2E).

## Phase 6 — Deploy (open)

1. Pick a host: Fly.io (`fly.toml` ready) or any Docker host.
2. Set `DATABASE_URL`, `SESSION_SECRET`, `BASE_URL` (https), `SMTP_URL`, `MAIL_FROM`.
3. Persistent volume for `VOICE_DIR`.
4. The image only contains `questdrop`; the `bot` binary needs its own build/service.

## Phase 7 — Hardening (later)

From the known limitations in [design.md §6](./design.md#6-errors--moderation): shared rate limiter for more than one instance, server-side voice duration check (the recorder enforces 10-60 s), rename `swap.created` to `drop.created`, score `time_bucket`/`energy`.

## History

2026-09-15: built in one day. TypeScript scaffold → Rust (Axum + sqlx), phases 0-4, then 16 audit rounds (validation, races, idempotency, error shapes, rate limits, supply chain), then B, C, auth, upload and a digest-pinned Dockerfile.
