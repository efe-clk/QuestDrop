> 🇹🇷 Türkçe için tıklayın: [README.tr.md](./README.tr.md)

# QuestDrop

Turn abandoned side-projects into tradeable quests. Tired of it? Drop it. Pick up someone else's half-finished quest and keep going with fresh novelty dopamine.

Success criterion: strangers trading in the open pool.

## Scope (MVP = A)

1. Lean web exchange. Build A first.
2. B (Discord/Telegram bot) and C (AI matching) plug in later, core stays unchanged.
3. Any project type: code, writing, design. No type field, link + voice only.

## Architecture

Single repo, API-first modular monolith. No microservices.

```text
[Web UI] -> [API Core] -> [DB]
[Bot]   -> [API Core] (later, same API)
```

- Core knows nothing about the outside world. Web and bot use the same REST API.
- Matching is one interface: `Matcher.match(user, pool) -> ranked_list`. Rule-based now, `EmbeddingMatcher` plugs into the same interface later.
- Event: simple DB outbox table for `swap.created`. Bot notifications and AI re-ranking listen to it.
- Auth: magic link. Files: link only (no S3). Voice: 30s mp3.

## Stack (locked)

- Web + API: Next.js App Router (TypeScript), REST
- DB: Postgres (Neon) + Prisma, pgvector column ready (NULL for now)
- Auth: Auth.js email magic link
- Voice storage: Vercel Blob
- Deploy: Vercel + Neon

Details: [docs/stack.md](./docs/stack.md) | [Türkçesi](./docs/stack.tr.md)

## Docs

- Design (EN): [docs/design.md](./docs/design.md)
- Tasarım (TR): [docs/design.tr.md](./docs/design.tr.md)

## Quickstart

```bash
npm install
npm run dev
```

> Scaffold landing in next commit. See design doc for flow.
