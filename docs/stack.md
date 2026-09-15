> 🇹🇷 Türkçe için tıklayın: [stack.tr.md](./stack.tr.md)

# Stack — Locked

Date: 2026-09-15

## Decision

- Web + API: Next.js App Router (TypeScript), REST route handlers. One repo, one deploy. Bot reuses the same endpoints later.
- DB: Postgres (Neon) + Prisma. Row-lock swap transaction + outbox table are easy. `embedding` column reserved via pgvector, NULL for now.
- Auth: Auth.js email magic link. 15 min setup, no passwords.
- Voice storage: Vercel Blob. 30s mp3 upload, stored as `voice_url`. No bucket management.
- Deploy: Vercel + Neon. 10 min deploy.

## Why

- Modular monolith stays intact: B/C plug into the same REST + outbox, no core change.
- Prisma transaction covers atomic swap (two offers -> matched).
- pgvector ready means C needs no migration, only a new `EmbeddingMatcher`.

## Non-goals

- No microservices, no GraphQL, no S3 setup in MVP.
