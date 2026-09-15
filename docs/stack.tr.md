> 🇬🇧 Click for English: [stack.md](./stack.md)

# Altyapı — Kilitli (Rust)

Tarih: 2026-09-15

## Karar

- Web + API: Axum (Rust), `/v1/` altında REST. Askama + HTMX ile server-render, Node yok. Tek repo, tek binary. Bot sonradan aynı endpointleri kullanır.
- DB: Postgres + sqlx. Row-lock takas transaction + outbox tablosu. `embedding` şimdilik JSONB placeholder; gerçek pgvector kolonu C fazıyla gelir.
- Auth: email magic link — PLANLANDI, henüz yok (MVP'de tüm endpointler açık).
- Ses depolama: MVP'de local `./data/voice`, static serve ile `voice_url`. S3 kurulumu yok. API değişmeden sonradan S3-uyumluya taşınır.
- Deploy: Docker tek binary + Fly.io (herhangi bir Docker host olur).
- API sözleşmesi: [openapi.yaml](./openapi.yaml) (elde yazıldı, kodu aynalar), RFC 9457 hatalar, swap'te `Idempotency-Key`, cursor sayfalama.

## Neden

- Modüler monolit korunur: B/C aynı REST + outbox'a takılır, core değişmez.
- sqlx transaction atomik takası kapsar (iki teklif -> matched), `SELECT ... FOR UPDATE` ile.
- pgvector C fazıyla gelir; o zamana kadar `EmbeddingMatcher: Matcher` impl için trait sabit.

## Kapsam Dışı

- MVP'de mikroservis yok, GraphQL yok, S3 kurulumu yok.
