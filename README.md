> 🇹🇷 Türkçe için tıklayın: [README.tr.md](./README.tr.md)

# QuestDrop (Rust)

Turn abandoned side-projects into tradeable quests. Tired of it? Drop it. Pick up someone else's half-finished quest and keep going with fresh novelty dopamine.

Success criterion: strangers trading in the open pool.

## Status (2026-10-12)

| Part | State |
|---|---|
| REST API (`/v1/`), Postgres schema, auth, voice upload | Done, 45 tests green (16 unit + 29 integration) |
| B: outbox publisher + `bot` binary (log / Discord webhook) | Done |
| C: `EmbeddingMatcher` (local trigram + cosine, `?algo=embedding`) | Done |
| Web UI (Askama + HTMX) | **Not built.** Only a placeholder home page exists. |
| Deploy | **Not deployed.** `Dockerfile` and `fly.toml` are ready. |

The API is nearly ready for a UI. Four gaps still block one; see [docs/plan.md](./docs/plan.md#phase-5--web-ui-open).

## Scope (MVP = A)

1. Lean web exchange. Build A first.
2. B (bot notifications) and C (smarter matching) plug in later; the core stays unchanged.
3. Any project type: code, writing, design. No type field, link + voice only.

## Architecture

Single repo, API-first modular monolith in Rust. No microservices.

```text
[Web UI - Askama+HTMX] -> [API Core - Axum] -> [Postgres+sqlx]
[bot binary] ----------> [outbox_events table] (polls the same DB)
```

- The core knows nothing about the outside world. A web UI and any future client use the same REST API (`/v1/`).
- Matching is one trait: `trait Matcher { fn match_items(user, pool) -> Vec<RankedItem> }`, with two implementations: `RuleMatcher` (default) and `EmbeddingMatcher`.
- Events go through an outbox table (`outbox_events`). A drop writes `swap.created` and a completed swap writes `swap.completed`. The `bot` binary delivers them.
- Auth: email magic link + HMAC-signed session cookie. Files: link only (no S3). Voice: mp3 under `VOICE_DIR`.

Details: [docs/design.md](./docs/design.md) · [docs/stack.md](./docs/stack.md) · [docs/plan.md](./docs/plan.md) · [docs/web-ui.md](./docs/web-ui.md) · API contract: [docs/openapi.yaml](./docs/openapi.yaml)

Turkish: [design.tr.md](./docs/design.tr.md) · [stack.tr.md](./docs/stack.tr.md) · [plan.tr.md](./docs/plan.tr.md) · [web-ui.tr.md](./docs/web-ui.tr.md)

## API

Login means a valid `qd_session` cookie. Every POST except `auth/callback` and `auth/logout` is limited to 10 requests/min per IP.

| Method | Path | Login | Notes |
|---|---|---|---|
| GET | `/` | – | placeholder HTML home with the open-pool count |
| GET | `/health` | – | always 200, `{"status":"ok","db":"up"\|"down"}` |
| GET | `/ready` | – | 200 when the DB answers, else 503 |
| GET | `/v1/projects?cursor=&limit=` | – | OPEN pool, newest first, cursor pages (default 20, max 50) |
| POST | `/v1/projects` | – | drop a quest; `handle` + `email` go in the body; max 3 per giver in any 24 h |
| POST | `/v1/users/upsert` | ✔ | your own skill profile (`can_do`, `looking_for`) |
| GET | `/v1/match?user_id=&algo=` | – | top-3 + 1 surprise; `algo=rule` (default) or `embedding` |
| POST | `/v1/swaps` | ✔ | atomic two-way trade of two offers; optional `Idempotency-Key` |
| GET | `/v1/revive?user_id=` | – | every completed take with its freeze package + first 2-minute task |
| POST | `/v1/reports` | ✔ | 3 reports from different users close the project |
| POST | `/v1/auth/request` | – | emails a login link (15-min single-use token) |
| POST | `/v1/auth/callback` | – | redeems the token and sets the session cookie (30 days) |
| POST | `/v1/auth/logout` | – | deletes the session (idempotent) |
| GET | `/v1/me` | ✔ | session identity |
| POST | `/v1/voice` | ✔ | mp3 upload (multipart field `file`, 5 MB cap) |
| GET | `/voice/*` | – | stored mp3 files |

Errors use RFC 9457 problem JSON. Unknown paths return a JSON 404, a wrong method returns a JSON 405, and a trailing slash redirects with 308.

## Quickstart

```bash
cp .env.example .env   # set DATABASE_URL to a reachable Postgres (13+)
cargo run              # migrations run automatically on boot
```

Open `http://localhost:3000`; `/health` shows the DB state. Without a database the server still starts: the pool is empty and writes return 503.

Without `SMTP_URL` the login link goes to the log (visible at `RUST_LOG=questdrop=info`) instead of an inbox.

Bot (outbox delivery): `cargo run --bin bot`.

## Tests

```bash
TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/questdrop_test" cargo test
```

The integration tests `TRUNCATE` every table. They refuse to run unless the database URL contains `test`.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | – | Postgres URL; unset = degraded mode |
| `PORT` | `3000` | listen port |
| `VOICE_DIR` | `./data/voice` | mp3 storage directory |
| `VOICE_QUOTA_MB` | `500` | uploads are refused (507) once the directory reaches this size |
| `SESSION_SECRET` | random per boot | at least 16 chars; required in production or sessions die on restart |
| `BASE_URL` | `http://localhost:3000` | base of login links; `https://` adds the `Secure` cookie flag |
| `SMTP_URL` / `MAIL_FROM` | – / `questdrop@localhost` | real mail; unset = log mailer |
| `RUST_LOG` | `questdrop=warn,tower_http=warn` | log filter |
| `BOT_SINK` / `BOT_WEBHOOK_URL` | `log` / – | bot only: `webhook` posts to a Discord webhook |
| `PUBLISHER_INTERVAL_SECS` | `5` | bot only: poll interval |
| `TEST_DATABASE_URL` | – | tests only |
