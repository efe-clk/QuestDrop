> 🇬🇧 Click for English: [plan.md](./plan.md)

# QuestDrop — Rust Yapım Planı

Durum (2026-10-12): **Backend bitti, web arayüzü açık, yayında değil.**
Faz 0-4 ile B ve C API seviyesinde yazıldı (50 test yeşil). Faz 1-4'ün kullanıcıya bakan yarısı (formlar ve sayfalar) hiç yapılmadı. Faz 5 bunu kapsar.

Stack: Axum + sqlx + Postgres + Askama/HTMX. Tek binary, `/v1/` REST.

## Faz 0 — İskelet ✅

1. `cargo init`, Axum router + `/health`, Askama baz layout
2. sqlx + Postgres bağlantısı, migration'lar açılışta koşar
3. Dockerfile + Fly.io config, `/voice` static serve

Bitti: `cargo run` ana sayfa + health sunar.

## Faz 1 — Bırak + Dondur ✅ API · ❌ UI

1. Tablolar: users, projects, swap_offers, outbox_events
2. `POST /v1/projects`: link + ses zorunlu, time_bucket + energy validasyonu
3. Dondur: proje (OPEN) + teklif (OPEN) + outbox olayı tek transaction'da

Bitti: API bırakmaları kaydeder, havuzda listeler. Eksik: bırakma formu.

## Faz 2 — Eşleş ✅ API · ❌ UI

1. `trait Matcher`, `RuleMatcher`: %80 yetenek uyumu + %20 rastgele (süre/enerji henüz skora girmiyor)
2. `GET /v1/match?user_id=`: top-3 + 1 sürpriz
3. Unit test: skor sırası ve aralığı

Eksik: eşleşme sayfası.

## Faz 3 — Atomik takas ✅ API · ❌ UI

1. `POST /v1/swaps`, `Idempotency-Key` ile, id sırasıyla `SELECT ... FOR UPDATE`
2. Tek transaction: iki teklif → MATCHED, match satırı, outbox olayı
3. Integration test: çift kapma engellenir, aynı anahtar yarışı 1×201 + 4×200 verir

Eksik: takas onay akışı.

## Faz 4 — Canlandır + Moderasyon ✅ API · ❌ UI

1. Canlandır: freeze paketi + ilk 2 dakika görevi
2. Kurallar: günde max 3 bırakma, kendinle eşleşme yok, 3 report = oto-gizle
3. RFC 9457 hatalar, cursor sayfalama

Eksik: canlandır sayfası, report butonu ve tarayıcıdan bırak → eşleş → al E2E'si.

## B / C ✅ (v1)

- B: outbox publisher + `bot` binary, log sink ya da Discord webhook sink.
- C: `EmbeddingMatcher` (yerel trigram + kosinüs), `?algo=embedding` ile.

## Faz 5 — Web arayüzü (açık)

Aynı binary içinde server-render Askama + HTMX sayfaları; böylece oturum cookie'si CORS'suz çalışır. Ayrıntılı arayüz tasarımı: [web-ui.tr.md](./web-ui.tr.md).

Sayfalar:

1. `/` havuz listesi (`GET /v1/projects`, cursor ile "daha fazla")
2. Bırakma formu + mp3 yükleme (önce `POST /v1/voice`, sonra `POST /v1/projects`)
3. Giriş: e-posta formu (`POST /v1/auth/request`) ve token'ı `/v1/auth/callback`'e POST eden **`/login?token=`**
4. Profil: `can_do` / `looking_for` (`POST /v1/users/upsert`)
5. "Al" butonlu eşleşme listesi (`GET /v1/match`, `POST /v1/swaps`)
6. Canlandır sayfası (`GET /v1/revive`)
7. Report butonu (`POST /v1/reports`) ve çıkış

Bu faz öncesinde ya da sırasında kapatılacak API eksikleri:

| # | Eksik | Arayüzü neden engelliyor |
|---|---|---|
| 1 | ✅ 2026-10-12: e-postadaki linkin hedefi ayarlanabilir (`LOGIN_URL`); `/login` sayfasının kendisi arayüz işi | link, token'ı POST eden bir sayfayı açmalı |
| 2 | ✅ 2026-10-12: havuz ve eşleşme öğelerinde `offer_id` var | `POST /v1/swaps` teklif id'si istiyor |
| 3 | ✅ 2026-10-12: `GET /v1/me/offers?status=` | alan kişi hangi quest'ini vereceğini seçer |
| 4 | CORS katmanı yok | başka bir origin'deki arayüz API'yi cookie ile çağıramaz; arayüzü bu binary'den sun ya da credentials'lı CORS ekle |

2026-10-12'de ayrıca yapıldı: ses yükleme tarayıcı kayıtlarını kabul ediyor (ogg, webm, m4a). Hâlâ açık: ana sayfa `/v1/match`'e `user_id` olmadan link veriyor (400 döner), ses süresi denetlenmiyor.

Bitti tanımı: tanımadığın biri tarayıcıdan bırakır, giriş yapar, eşleşir, takas eder ve canlandırır.

## Faz 6 — Deploy (açık)

1. Host seç: Fly.io (`fly.toml` hazır) ya da herhangi bir Docker host.
2. `DATABASE_URL`, `SESSION_SECRET`, `BASE_URL` (https), `SMTP_URL`, `MAIL_FROM` ayarla.
3. `VOICE_DIR` için kalıcı volume.
4. İmajda sadece `questdrop` var; `bot` binary'si ayrı bir build/servis ister.

## Faz 7 — Sağlamlaştırma (sonra)

[design.tr.md §6](./design.tr.md#6-hata--moderasyon) içindeki bilinen sınırlardan: `/v1/match` ve `/v1/revive`'ı login'e bağla, bırakmada login kararını ver, birden çok instance için ortak rate limiter, ses süresi denetimi, `swap.created` adını `drop.created` yap, `time_bucket`/`energy`'yi skora kat.

## Geçmiş

2026-09-15: tek günde yapıldı. TypeScript iskelet → Rust (Axum + sqlx), faz 0-4, ardından 16 denetim turu (validasyon, yarışlar, idempotency, hata biçimleri, rate limit, tedarik zinciri), sonra B, C, auth, yükleme ve digest-pinli Dockerfile.
