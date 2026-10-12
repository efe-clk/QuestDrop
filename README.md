> 🇹🇷 Türkçe için tıklayın: [README.tr.md](./README.tr.md)

# QuestDrop (Rust)

Turn abandoned side-projects into tradeable quests. Tired of it? Drop it. Pick up someone else's half-finished quest and keep going with fresh novelty dopamine.

Success criterion: strangers trading in the open pool.

## Status (2026-10-12)

| Part | State |
|---|---|
| REST API (`/v1/`), Postgres schema, auth, voice upload | Done |
| B: outbox publisher + `bot` binary (log / Discord webhook) | Done |
| C: `EmbeddingMatcher` (local trigram + cosine, `?algo=embedding`) | Done |
| Web UI (Askama + HTMX, English + Turkish) | Done: pool, login, drop with in-browser voice recording, match, swap, revive, profile, report |
| Tests | 64 Rust tests green (25 unit + 31 API + 8 web) plus a Playwright browser E2E |
| Deploy | **Not deployed.** `Dockerfile` and `fly.toml` are ready. |

The web UI follows [docs/web-ui.md](./docs/web-ui.md); every page works without JavaScript and HTMX only speeds it up.

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
- Auth: email magic link + HMAC-signed session cookie. Files: link only (no S3). Voice: mp3, ogg, webm or m4a under `VOICE_DIR`.

Details: [docs/design.md](./docs/design.md) · [docs/stack.md](./docs/stack.md) · [docs/plan.md](./docs/plan.md) · [docs/web-ui.md](./docs/web-ui.md) · API contract: [docs/openapi.yaml](./docs/openapi.yaml)

Turkish: [design.tr.md](./docs/design.tr.md) · [stack.tr.md](./docs/stack.tr.md) · [plan.tr.md](./docs/plan.tr.md) · [web-ui.tr.md](./docs/web-ui.tr.md)

## API

Login means a valid `qd_session` cookie. Every POST except `auth/callback` and `auth/logout` is limited to 10 requests/min per IP.

| Method | Path | Login | Notes |
|---|---|---|---|
| GET | `/`, `/login`, `/q/{id}` | – | web UI pages (HTML) |
| GET | `/drop`, `/match`, `/take/{offer_id}`, `/revive`, `/me` | ✔ | web UI pages (HTML; logged out → `/login?next=…`) |
| GET | `/health` | – | always 200, `{"status":"ok","db":"up"\|"down"}` |
| GET | `/ready` | – | 200 when the DB answers, else 503 |
| GET | `/v1/projects?cursor=&limit=` | – | OPEN pool, newest first, cursor pages (default 20, max 50); items carry `offer_id` |
| GET | `/v1/projects/{id}` | – | one project (OPEN or MATCHED) |
| POST | `/v1/projects` | ✔ | drop a quest as the session user; max 3 per giver in any 24 h |
| POST | `/v1/users/upsert` | ✔ | your own skill profile (`can_do`, `looking_for`) |
| GET | `/v1/match?algo=` | ✔ | top-3 + 1 surprise for the session user, with `offer_id`; `algo=rule` (default) or `embedding` |
| POST | `/v1/swaps` | ✔ | atomic two-way trade of two offers; optional `Idempotency-Key` |
| GET | `/v1/revive` | ✔ | your completed takes with their freeze package + first 2-minute task |
| POST | `/v1/reports` | ✔ | 3 reports from different users close the project |
| POST | `/v1/auth/request` | – | emails a login link (15-min single-use token) |
| POST | `/v1/auth/callback` | – | redeems the token and sets the session cookie (30 days) |
| POST | `/v1/auth/logout` | – | deletes the session (idempotent) |
| GET | `/v1/me` | ✔ | session identity |
| GET | `/v1/me/offers?status=` | ✔ | your own offers (pick what to give in a swap); `status` = open, matched or closed |
| POST | `/v1/voice` | ✔ | audio upload: mp3, ogg, webm or m4a (multipart field `file`, 5 MB cap) |
| GET | `/voice/*` | – | stored voice files |

Errors use RFC 9457 problem JSON. Unknown paths return a JSON 404, a wrong method returns a JSON 405, and a trailing slash redirects with 308.

## Quickstart

```bash
cp .env.example .env   # set DATABASE_URL to a reachable Postgres (13+)
cargo run              # migrations run automatically on boot
```

Open `http://localhost:3000`; `/health` shows the DB state. Without a database the server still starts: the pool is empty and writes return 503.

Without `SMTP_URL` the login link goes to the server log (at warn level, so it shows with the default filter) instead of an inbox.

Bot (outbox delivery): `cargo run --bin bot`.

## Tests

```bash
TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/questdrop_test" cargo test
```

The integration tests `TRUNCATE` every table. They refuse to run unless the database URL contains `test`.

Browser E2E (Playwright, needs a running server and a fresh database): see [e2e/README.md](./e2e/README.md).

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | – | Postgres URL; unset = degraded mode |
| `PORT` | `3000` | listen port |
| `VOICE_DIR` | `./data/voice` | voice note storage directory |
| `STATIC_DIR` | `./static` | CSS/JS/HTMX for the web UI |
| `VOICE_QUOTA_MB` | `500` | uploads are refused (507) once the directory reaches this size |
| `SESSION_SECRET` | random per boot | at least 16 chars; required in production or sessions die on restart |
| `BASE_URL` | `http://localhost:3000` | base of login links; `https://` adds the `Secure` cookie flag |
| `LOGIN_URL` | `BASE_URL/login` | page the emailed link opens (`?token=` is appended); point it at the UI's login page |
| `SMTP_URL` / `MAIL_FROM` | – / `questdrop@localhost` | real mail; unset = log mailer |
| `RUST_LOG` | `questdrop=warn,tower_http=warn` | log filter |
| `BOT_SINK` / `BOT_WEBHOOK_URL` | `log` / – | bot only: `webhook` posts to a Discord webhook |
| `PUBLISHER_INTERVAL_SECS` | `5` | bot only: poll interval |
| `TEST_DATABASE_URL` | – | tests only |
