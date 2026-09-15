> 🇬🇧 Click for English: [stack.md](./stack.md)

# Altyapı — Kilitli

Tarih: 2026-09-15

## Karar

- Web + API: Next.js App Router (TypeScript), REST route handler. Tek repo, tek deploy. Bot sonradan aynı endpointleri kullanır.
- DB: Postgres (Neon) + Prisma. Row-lock takas transaction + outbox tablosu kolay. `embedding` kolonu pgvector ile rezerve, şimdilik NULL.
- Auth: Auth.js email magic link. 15 dk kurulum, şifre yok.
- Ses depolama: Vercel Blob. 30sn mp3 yüklenir, `voice_url` olarak saklanır. Bucket yönetimi yok.
- Deploy: Vercel + Neon. 10 dk deploy.

## Neden

- Modüler monolit korunur: B/C aynı REST + outbox'a takılır, core değişmez.
- Prisma transaction atomik takası kapsar (iki teklif -> matched).
- pgvector hazır demek C için migration gerekmez, sadece yeni `EmbeddingMatcher`.

## Kapsam Dışı

- MVP'de mikroservis yok, GraphQL yok, S3 kurulumu yok.
