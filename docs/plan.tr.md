> 🇬🇧 Click for English: [plan.md](./plan.md)

# QuestDrop — Rust Yapım Planı

Durum (2026-10-12): **Backend ve web arayüzü bitti, yayında değil.**
Faz 0-5 ile B ve C yazıldı: API ve üstündeki tarayıcı sayfaları (59 Rust testi + tarayıcı E2E'si, hepsi yeşil).

Stack: Axum + sqlx + Postgres + Askama/HTMX. Tek binary, `/v1/` REST.

## Faz 0 — İskelet ✅

1. `cargo init`, Axum router + `/health`, Askama baz layout
2. sqlx + Postgres bağlantısı, migration'lar açılışta koşar
3. Dockerfile + Fly.io config, `/voice` static serve

Bitti: `cargo run` ana sayfa + health sunar.

## Faz 1 — Bırak + Dondur ✅

1. Tablolar: users, projects, swap_offers, outbox_events
2. `POST /v1/projects`: link + ses zorunlu, time_bucket + energy validasyonu
3. Dondur: proje (OPEN) + teklif (OPEN) + outbox olayı tek transaction'da

Bitti: API bırakmaları kaydeder, havuzda listeler; web bırakma formu (faz 5) bunu kullanır.

## Faz 2 — Eşleş ✅

1. `trait Matcher`, `RuleMatcher`: %80 yetenek uyumu + %20 rastgele (süre/enerji henüz skora girmiyor)
2. `GET /v1/match?user_id=`: top-3 + 1 sürpriz
3. Unit test: skor sırası ve aralığı

Arayüz: `/match`.

## Faz 3 — Atomik takas ✅

1. `POST /v1/swaps`, `Idempotency-Key` ile, id sırasıyla `SELECT ... FOR UPDATE`
2. Tek transaction: iki teklif → MATCHED, match satırı, outbox olayı
3. Integration test: çift kapma engellenir, aynı anahtar yarışı 1×201 + 4×200 verir

Arayüz: `/take/{offer_id}`.

## Faz 4 — Canlandır + Moderasyon ✅

1. Canlandır: freeze paketi + ilk 2 dakika görevi
2. Kurallar: günde max 3 bırakma, kendinle eşleşme yok, 3 report = oto-gizle
3. RFC 9457 hatalar, cursor sayfalama

Arayüz: `/revive` ve her kartta şikâyet menüsü.

## B / C ✅ (v1)

- B: outbox publisher + `bot` binary, log sink ya da Discord webhook sink.
- C: `EmbeddingMatcher` (yerel trigram + kosinüs), `?algo=embedding` ile.

## Faz 5 — Web arayüzü ✅

Aynı binary içinde server-render Askama + HTMX sayfaları (`src/web.rs`, `templates/`, `static/`); oturum cookie'si CORS'suz çalışır. Sözleşme: [web-ui.tr.md](./web-ui.tr.md). Her form JavaScript olmadan da çalışır; HTMX (`hx-boost`) sadece hızlandırır.

Sayfalar: `/` havuz (cursor ile "daha fazla göster"), `/login` (+ `?token=` onay butonu), `/drop` (tarayıcıdan kayıt + dosya yükleme), `/match`, `/take/{offer_id}`, `/revive` (aldıklarım / bıraktıklarım, ilk 2 dakika), `/me`, `/q/{id}` (paylaşılabilir, Open Graph), kartlarda şikâyet menüsü, çıkış. İngilizce + Türkçe (`i18n/*.toml`).

Bunun için kapatılan API eksikleri (web-ui §10): `/login` sayfası; havuz, eşleşme ve proje satırlarında `offer_id`; `GET /v1/me/offers`; `GET /v1/projects/{id}`; ses mp3'ün yanında Opus (webm/ogg) ve AAC (m4a) kabul eder; bırakma, `/v1/match` ve `/v1/revive` login ister; ölü `/v1/match` linki kaldırıldı. CORS gerekmiyor (aynı origin).

Testler: `tests/web.rs` (sayfalar, giriş akışı, CSRF, bırakma validasyonu + günlük sınır + JS'siz yükleme, eşleş → al → canlandır, idempotent çift gönderim, yarış durumu, şikâyet + otomatik gizleme, sayfalama parçası, DB yok) ve `e2e/` (Playwright: iki kullanıcı, sahte mikrofonla kayıt, gerçek yarış, axe, 360/768/1280 açık/koyu ekran görüntüleri).

Bitti tanımı: tanımadığın biri tarayıcıdan bırakır, giriş yapar, eşleşir, takas eder ve canlandırır: ✅ (E2E kapsıyor).

## Faz 6 — Deploy (açık)

1. Host seç: Fly.io (`fly.toml` hazır) ya da herhangi bir Docker host.
2. `DATABASE_URL`, `SESSION_SECRET`, `BASE_URL` (https), `SMTP_URL`, `MAIL_FROM` ayarla.
3. `VOICE_DIR` için kalıcı volume.
4. İmajda sadece `questdrop` var; `bot` binary'si ayrı bir build/servis ister.

## Faz 7 — Sağlamlaştırma (sonra)

[design.tr.md §6](./design.tr.md#6-hata--moderasyon) içindeki bilinen sınırlardan: birden çok instance için ortak rate limiter, sunucu tarafında ses süresi denetimi (kaydedici 10-60 sn'yi uygular), `swap.created` adını `drop.created` yap, `time_bucket`/`energy`'yi skora kat.

## Geçmiş

2026-09-15: tek günde yapıldı. TypeScript iskelet → Rust (Axum + sqlx), faz 0-4, ardından 16 denetim turu (validasyon, yarışlar, idempotency, hata biçimleri, rate limit, tedarik zinciri), sonra B, C, auth, yükleme ve digest-pinli Dockerfile.
