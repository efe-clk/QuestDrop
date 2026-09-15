> 🇬🇧 Click for English: [stack.md](./stack.md)

# Altyapı — Kilitli (Rust)

Tarih: 2026-09-15

## Karar

- Web + API: Axum (Rust), `/v1/` altında REST. Askama + HTMX ile server-render, Node yok. Tek repo, tek binary. Bot sonradan aynı endpointleri kullanır.
- DB: Postgres + sqlx. Row-lock takas transaction + outbox tablosu. `embedding` kolonu pgvector ile rezerve, şimdilik NULL.
- Auth: email magic link (lettre), imzalı session cookie. Şifre yok.
- Ses depolama: MVP'de local `./data/voice`, static serve ile `voice_url`. S3 kurulumu yok. API değişmeden sonradan S3-uyumluya taşınır.
- Deploy: Docker tek binary + Fly.io (herhangi bir Docker host olur).
- API sözleşmesi: OpenAPI 3.x, RFC 9457 hatalar, swap'te `Idempotency-Key`, cursor sayfalama.

## Neden

- Modüler monolit korunur: B/C aynı REST + outbox'a takılır, core değişmez.
- sqlx transaction atomik takası kapsar (iki teklif -> matched), `SELECT ... FOR UPDATE` ile.
- pgvector hazır demek C için migration gerekmez, sadece yeni `EmbeddingMatcher: Matcher` impl.

## Kapsam Dışı

- MVP'de mikroservis yok, GraphQL yok, S3 kurulumu yok.
