> 🇹🇷 Türkçe için tıklayın: [design.tr.md](./design.tr.md)

# QuestDrop — Design Document (Rust)

Date: 2026-09-15 · Last checked against the code: 2026-10-12
Status: Backend implemented; web UI not built yet (see [plan.md](./plan.md))

## 1. Goal

Turn quitting from shame into a game. Tired? Drop the project; pick up someone else's half-finished quest and continue with novelty dopamine. Breaks the abandonment loop in ADHD.

Success criterion: strangers trading in the open pool.

## 2. Scope (MVP = A)

- Lean web exchange. Build A first.
- B (bot notifications) and C (smarter matching) plug in later; the core stays unchanged. Both now exist in a first version (§8).
- Any project type: code, writing, design. No type field, link + voice only.

## 3. Architecture

Single repo, API-first modular monolith. No microservices.

```text
[Web UI] -----> [API Core /v1/] -> [Postgres]
[bot binary] -> [outbox_events] (polls the same DB)
```

- The core knows nothing about the outside world. Every client uses the same REST API.
- Matching is one trait: `trait Matcher { fn match_items(user, pool) -> Vec<RankedItem> }`.
- Events: DB outbox table (`outbox_events`), polled by the `bot` binary.
- Auth: email magic link + server-side sessions (§4).
- Files: link only (no S3). Voice: mp3, either uploaded to `/v1/voice` or an external http(s) URL ending in `.mp3`.

## 4. Components + Data Model

1. **users**: `id, handle (unique), email (unique), can_do[], looking_for[]`. A user is created on first drop or first login.
2. **projects** (freeze package): `title (3-80), one_liner (10-200), link_url, voice_url, skill_needed[] (max 10, each 1-32), time_bucket S|M|L, energy LOW|MID|HIGH, status OPEN|MATCHED|CLOSED, giver_id, embedding JSONB (unused placeholder)`.
   - `time_bucket` means S = under 2 h, M = 2-8 h, L = over 8 h.
3. **swap_offers**: `project_id, giver_id, status OPEN|MATCHED|CLOSED`. Each drop creates exactly one offer. **Swaps work on offer ids, not project ids.**
4. **matches**: `taker_id, given_id, taken_id (project ids), score, reason`.
5. **reports**: `project_id, reporter_id, reason (10-300)`, one per reporter per project.
6. **magic_tokens** (SHA-256 hash only, 15 min, single use) and **sessions** (30 days).
7. **idempotency_keys**: stored swap responses per `(key, taker)`.

Out of scope: comments, likes, DMs, scores, category tree.

### Identity rules

- A drop carries `handle` + `email` in the body and needs no login. An existing email must come with its own handle, or the drop gets 409. A new pair creates the user.
- Login: `POST /v1/auth/request` stores a token and sends `BASE_URL/login?token=…`. A client page at `/login` must POST that token to `/v1/auth/callback`, which sets the `qd_session` cookie (`HttpOnly; SameSite=Lax`, plus `Secure` when `BASE_URL` is https). **The `/login` page does not exist yet.**
- Swaps, reports, profile edits and voice uploads need a session, and the body ids must match the session user (else 403).

## 5. Data Flow

1. **Drop**: form → validation (§4) → if `voice_url` is a `/voice/…` path, the file must exist.
2. **Freeze**: one transaction creates the project (OPEN), its offer (OPEN) and an outbox `swap.created` event. Max 3 drops per giver in any rolling 24 h (429 with `Retry-After`).
3. **Match**: `GET /v1/match?user_id=` ranks up to 100 of the newest OPEN projects that aren't the user's own, and returns top-3 + 1 surprise.
   - `RuleMatcher` (default): score = 0.8 × skill fit + 0.2 × random. Skill fit is the average overlap of `skill_needed` with the user's `can_do` and `looking_for`. **`time_bucket` and `energy` are not scored yet.**
   - `EmbeddingMatcher` (`algo=embedding`): deterministic trigram-hash cosine over the same skill words; returns the top 4, no random term.
4. **Swap**: a **two-way trade**. The taker gives one of their own OPEN offers and takes someone else's OPEN offer. One transaction locks both offers (in id order), sets both offers and projects to MATCHED, writes a `matches` row with the deterministic skill fit as `score`, and an outbox `swap.completed` event. A user who has not dropped anything cannot take.
5. **Revive**: `GET /v1/revive?user_id=` lists every project the user has taken, newest first, each with its freeze package and a fixed 4-step "first 2 minutes" task.

## 6. Errors / Moderation

- Missing or invalid voice/link: no save, and the response lists every failed rule.
- Self-swap and taking your own project are refused (409).
- Max 3 drops per day. 3 distinct reports set the project and its open offers to CLOSED (auto-hide). The reports table is the review queue; there is no review UI yet. No deletes, only closing.
- No AI fallback is needed: `EmbeddingMatcher` is local and deterministic, and `rule` stays the default.
- The core keeps running when the bot is down; events wait in the outbox.

### Known limitations (2026-10-12)

- Voice duration (meant to be 10-60 s) is **not** checked. Only format (magic bytes / `.mp3`) and size (5 MB) are.
- `/v1/match` and `/v1/revive` take any `user_id` without a login, so anyone with a user id can read that user's matches and taken projects.
- A drop needs no login, so anyone who knows a user's handle + email can drop in their name.
- The rate limiter is in memory and per process (resets on restart, not shared across instances). It trusts the leftmost `X-Forwarded-For` value.
- The event name `swap.created` is emitted for a **drop**, not a swap. Rename it before external consumers depend on it.

## 7. Tests

- Unit (16): validation, matcher ordering and range, auth helpers, first task, Discord body.
- Integration (29, real Postgres): drop/pool/cap, swap atomicity and races (5 parallel same-key requests give 1×201 + 4×200), idempotency, reports and auto-hide, auth and cookies, voice upload, error shapes.
- E2E through a browser: not possible until the web UI exists. No load test.

## 8. B/C Extension Ports

- **B (done, v1):** `bot` binary polls `outbox_events` (`FOR UPDATE SKIP LOCKED`, at-least-once) and delivers to `LogSink` or `WebhookSink` (Discord incoming webhook). No Telegram sink yet.
- **C (done, v1):** `EmbeddingMatcher` behind the same trait. The `embedding` column is still unused; a pgvector-backed matcher can replace it without touching callers.
