> 🇹🇷 Türkçe için tıklayın: [README.tr.md](./README.tr.md)

# QuestDrop (Rust)

Turn abandoned side-projects into tradeable quests. Tired of it? Drop it. Pick up someone else's half-finished quest and keep going with fresh novelty dopamine.

Success criterion: strangers trading in the open pool.

## Scope (MVP = A)

1. Lean web exchange. Build A first.
2. B (Discord/Telegram bot) and C (AI matching) plug in later, core stays unchanged.
3. Any project type: code, writing, design. No type field, link + voice only.

## Architecture

Single repo, API-first modular monolith in Rust. No microservices.

```text
[Web UI - Askama+HTMX] -> [API Core - Axum] -> [Postgres+sqlx]
[Bot] -----------------> [API Core - Axum] (later, same API)
```

- Core knows nothing about the outside world. Web and bot use the same REST API (`/v1/`).
- Matching is one trait: `trait Matcher { fn match(...) -> ranked_list }`. Rule-based now, `EmbeddingMatcher` impl later.
- Event: simple DB outbox table for `swap.created`. Bot notifications and AI re-ranking listen to it.
- Auth: email magic link. Files: link only (no S3). Voice: 30s mp3 in `./data/voice`.

## Stack (locked - Rust)

- Web + API: Axum + Askama + HTMX, single binary
- DB: Postgres + sqlx, pgvector reserved
- Auth: magic link (lettre) + cookie session
- Voice: local static files
- Deploy: Docker + Fly.io

Details: [docs/stack.md](./docs/stack.md) | [Türkçesi](./docs/stack.tr.md)

## Docs

- Design (EN): [docs/design.md](./docs/design.md)
- Plan (EN): [docs/plan.md](./docs/plan.md)
- Tasarım (TR): [docs/design.tr.md](./docs/design.tr.md)
- Plan (TR): [docs/plan.tr.md](./docs/plan.tr.md)

## API

| Method | Path | Notes |
|---|---|---|
| GET | `/`, `/health`, `/ready` | home, liveness+db, readiness probe |
| GET | `/v1/projects?cursor=&limit=` | OPEN pool, cursor pages (max 50) |
| POST | `/v1/projects` | drop: validation, 3/day, 10/min per IP |
| POST | `/v1/users/upsert` | skill profile for matching (login) |
| GET | `/v1/match?user_id=` | top-3 + 1 surprise (RuleMatcher) |
| POST | `/v1/swaps` | atomic take; `Idempotency-Key` optional (login) |
| GET | `/v1/revive?user_id=` | freeze package + first 2-min task |
| POST | `/v1/reports` | 3 distinct reports auto-hide (login) |
| GET | `/voice/*` | local mp3 files |
| POST | `/v1/auth/request` | magic-link email (15-min token) |
| POST | `/v1/auth/callback` | redeem token → HttpOnly session cookie |
| GET | `/v1/me` | session identity (401 without cookie) |
| POST | `/v1/voice` | mp3 upload, 5MB cap (login) |

## Quickstart (5 min)

```bash
cp .env.example .env   # set DATABASE_URL to a reachable Postgres
cargo run              # migrations run automatically on boot
```

Open `http://localhost:3000`, health at `/health` (includes `db` status).
Without a database the server runs degraded: pool is empty, writes return 503.
