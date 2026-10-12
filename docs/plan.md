> 🇹🇷 Türkçe için tıklayın: [plan.tr.md](./plan.tr.md)

# QuestDrop — Rust Build Plan

Status (2026-10-12): **Backend done, web UI open, not deployed.**
Phases 0-4 plus B and C are implemented at the API level (45 tests green). The user-facing half of phases 1-4 (forms and pages) was never built. Phase 5 covers it.

Stack: Axum + sqlx + Postgres + Askama/HTMX. Single binary, `/v1/` REST.

## Phase 0 — Scaffold ✅

1. `cargo init`, Axum router + `/health`, Askama base layout
2. sqlx + Postgres connect, migrations run on boot
3. Dockerfile + Fly.io config, `/voice` static serve

Done: `cargo run` serves the home page + health.

## Phase 1 — Drop + Freeze ✅ API · ❌ UI

1. Tables: users, projects, swap_offers, outbox_events
2. `POST /v1/projects`: link + voice required, time_bucket + energy validation
3. Freeze: project (OPEN) + offer (OPEN) + outbox event in one transaction

Done: the API saves drops and lists them in the pool. Missing: the drop form.

## Phase 2 — Match ✅ API · ❌ UI

1. `trait Matcher`, `RuleMatcher`: 80% skill fit + 20% random (time/energy not scored yet)
2. `GET /v1/match?user_id=`: top-3 + 1 surprise
3. Unit test: score ordering and range

Missing: the match page.

## Phase 3 — Atomic swap ✅ API · ❌ UI

1. `POST /v1/swaps` with `Idempotency-Key`, `SELECT ... FOR UPDATE` in id order
2. One transaction: two offers → MATCHED, match row, outbox event
3. Integration test: double-claim blocked, same-key race gives 1×201 + 4×200

Missing: the swap confirmation flow.

## Phase 4 — Revive + Moderation ✅ API · ❌ UI

1. Revive: freeze package + first 2-minute task
2. Rules: max 3 drops/day, no self-match, 3 reports = auto-hide
3. RFC 9457 errors, cursor pagination

Missing: the revive page, the report button, and a browser E2E for drop → match → take.

## B / C ✅ (v1)

- B: outbox publisher + `bot` binary, log sink or Discord webhook sink.
- C: `EmbeddingMatcher` (local trigram + cosine) via `?algo=embedding`.

## Phase 5 — Web UI (open)

Server-rendered Askama + HTMX pages in the same binary, so the session cookie works without CORS. Full UI design: [web-ui.md](./web-ui.md).

Pages:

1. `/` pool list (`GET /v1/projects`, cursor "load more")
2. Drop form + mp3 upload (`POST /v1/voice` then `POST /v1/projects`)
3. Login: email form (`POST /v1/auth/request`) and **`/login?token=`**, which POSTs the token to `/v1/auth/callback`
4. Profile: `can_do` / `looking_for` (`POST /v1/users/upsert`)
5. Match list with a "take" button (`GET /v1/match`, `POST /v1/swaps`)
6. Revive page (`GET /v1/revive`)
7. Report button (`POST /v1/reports`) and logout

API gaps to close before or during this phase:

| # | Gap | Why it blocks the UI |
|---|---|---|
| 1 | No `/login` route | the emailed link lands on a JSON 404 |
| 2 | Pool, match and revive responses carry **project** ids, but `POST /v1/swaps` needs **offer** ids | the UI cannot build a swap request; add `offer_id` to pool/match items |
| 3 | No "my open offers" endpoint (only the drop response returns `offer_id`) | the taker cannot pick which of their quests to give |
| 4 | No CORS layer | a UI on another origin cannot call the API with cookies; serve the UI from this binary or add CORS with credentials |

Also: the home page links to `/v1/match` without `user_id` (returns 400), and voice duration is not checked.

Done when a stranger can drop, log in, match, swap and revive in a browser.

## Phase 6 — Deploy (open)

1. Pick a host: Fly.io (`fly.toml` ready) or any Docker host.
2. Set `DATABASE_URL`, `SESSION_SECRET`, `BASE_URL` (https), `SMTP_URL`, `MAIL_FROM`.
3. Persistent volume for `VOICE_DIR`.
4. The image only contains `questdrop`; the `bot` binary needs its own build/service.

## Phase 7 — Hardening (later)

From the known limitations in [design.md §6](./design.md#6-errors--moderation): login-gate `/v1/match` and `/v1/revive`, decide on login for drops, shared rate limiter for more than one instance, voice duration check, rename `swap.created` to `drop.created`, score `time_bucket`/`energy`.

## History

2026-09-15: built in one day. TypeScript scaffold → Rust (Axum + sqlx), phases 0-4, then 16 audit rounds (validation, races, idempotency, error shapes, rate limits, supply chain), then B, C, auth, upload and a digest-pinned Dockerfile.
