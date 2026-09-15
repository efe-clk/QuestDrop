> 🇬🇧 Click for English: [plan.md](./plan.md)

# QuestDrop — Rust Yapım Planı

Durum: Faz 0-2 bitti, Faz 3 takas bitti (atomik + idempotent, 24 test yeşil).

Stack: Axum + sqlx + Postgres + Askama/HTMX. Tek binary, `/v1/` REST.

## Faz 0 — İskelet (4 saat)

1. `cargo init`, Axum router + `/health`, Askama baz layout
2. sqlx + Postgres bağlantısı, `sqlx migrate` kurulumu
3. Dockerfile + Fly.io config, `./data/voice` static serve

Bitti: `cargo run` home + health sunar.

## Faz 1 — Bırak + Dondur (1 gün)

1. Tablolar: users, projects, swap_offers, outbox
2. `POST /v1/projects`: 30sn form, link + ses zorunlu, time_bucket + energy validation
3. Dondur: tek transaction ile project (open) + swap_offer (open) yaz

Bitti: bırakma formu kaydeder, havuzda listelenir.

## Faz 2 — Eşleş (1 gün)

1. `trait Matcher`, `RuleMatcher`: %80 uyum (yetenek + süre + enerji) + %20 rastgele
2. `GET /v1/match?user=`: top-3 + 1 sürpriz
3. Unit test: matcher skor sıralaması

Bitti: havuz 4'lü sıralı döner.

## Faz 3 — Atomik takas (1 gün)

1. `POST /v1/swaps`, `Idempotency-Key` ile, `SELECT ... FOR UPDATE`
2. Tek transaction: iki teklif -> matched, match satırı yaz, outbox'a yaz
3. Integration test: çift kapma engellenir

Bitti: iki kullanıcı yarışsız takas yapar.

## Faz 4 — Canlandır + Moderasyon (yarım gün)

1. Canlandır görünümü: freeze paketi + ilk 2 dakika görevi
2. Kurallar: günde max 3 bırakma, kendinle eşleşme yok, 3 report = oto-gizle
3. RFC 9457 hatalar, havuzda cursor sayfalama

Bitti: E2E bırak -> eşleş -> al geçer.

## B/C kapıları (sonra)

- B: bot aynı `/v1/`'i çağırır, outbox dinler.
- C: `EmbeddingMatcher: Matcher`, pgvector kolonu dolar.
