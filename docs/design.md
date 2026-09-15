> 🇹🇷 Türkçe için tıklayın: [design.tr.md](./design.tr.md)

# QuestDrop — Design Document (Rust)

Date: 2026-09-15
Status: Draft, Rust stack locked

## 1. Goal

Turn quitting from shame into a game. Tired? Drop the project; pick up someone else's half-finished quest and continue with novelty dopamine. Breaks the abandonment loop in ADHD.

Success criterion: strangers trading in the open pool.

## 2. Scope (MVP = A)

- Lean web exchange. Build A first.
- B (Discord/Telegram bot) and C (AI matching) plug in later, core stays unchanged.
- Any project type: code, writing, design. No type field, link + voice only.

## 3. Architecture

Single repo, API-first modular monolith. No microservices.

```text
[Web UI] -> [API Core] -> [DB]
[Bot] ----> [API Core] (later, same API)
```

- Core knows nothing about the outside world. Web and bot use the same REST API.
- Matching is one trait: `trait Matcher { fn match(user, pool) -> ranked_list }`. Rule-based now, `EmbeddingMatcher` impl later.
- Event: simple DB outbox table for `swap.created` (sqlx, polled publisher). Bot notifications and AI re-ranking listen to it.
- Auth: magic link. Files: link (no S3). Voice: 30s mp3.

## 4. Components + Data Model

1. Identity: id, handle, email, can_do[], looking_for[]
2. Project (Freeze Package): title, one_liner, link_url, voice_url (10-60s required), skill_needed[], time_bucket [S=<2h / M=2-8h / L=8h+], energy [low/mid/high], status, embedding NULL
3. Pool/Swap: swap_offers (project_id, giver_id, status open/matched/closed) + matches (taker_id, given, taken, score, reason)
4. Adapter: Web now, Bot later. No extra tables.

Out: comments, likes, DMs, scores, category tree. Moderation only: reports (project_id, reason).

## 5. Data Flow

1. Drop: 30s form, validation required.
2. Freeze: projects (open) + swap_offers (open).
3. Match: top-3 + 1 surprise, user picks. Score = 80% fit + 20% random.
4. Swap: single transaction, two offers matched, row-lock prevents double-claim.
5. Revive: receiver gets freeze package + first 2-minute task.

## 6. Errors / Moderation

- No voice/link = no save, explain why. No self-matching.
- Adapter crash keeps core alive. AI matcher failure falls back to rule-based.
- Max 3 drops per day. 3 reports = auto-hide + review queue. No delete, only close.

## 7. Tests

- Unit: matcher score.
- Integration: swap atomicity, API+DB.
- E2E: drop -> match -> take. No load test.

## 8. B/C Extension Ports

- B: new bot adapter, same endpoints, listens to events.
- C: embedding field ready, Matcher interface fixed, plugs in once data accumulates.
