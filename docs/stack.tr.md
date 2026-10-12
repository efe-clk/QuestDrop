> 🇬🇧 Click for English: [stack.md](./stack.md)

# Altyapı — Kilitli (Rust)

Tarih: 2026-09-15 · Kodla son karşılaştırma: 2026-10-12

## Karar

- Web + API: Axum 0.7 (Rust), `/v1/` altında REST. Askama + HTMX ile server-render, Node yok. Tek repo, tek binary. **Şimdiye kadar tek bir Askama şablonu var (`templates/index.html`), HTMX henüz yüklenmiyor**; bkz. [plan.tr.md faz 5](./plan.tr.md#faz-5--web-arayüzü-açık).
- DB: Postgres 13+ (yerleşik `gen_random_uuid()` kullanılır) + sqlx 0.8, sadece çalışma zamanı sorgularıyla; derlemek için veritabanı gerekmez. `migrations/` açılışta koşar. Row-lock takas transaction + outbox tablosu. `embedding` kullanılmayan bir JSONB yer tutucu; ileride yerine pgvector kolonu gelebilir. C fazının v1'i onu kullanmıyor.
- Auth: e-posta magic link (15 dk tek kullanımlık token, HMAC imzalı 30 günlük oturum, varsayılan LogMailer, `SMTP_URL` ile SMTP). Takas, report, profil düzenleme ve ses yükleme login ister; bırakma açık.
- Ses depolama: yerel `VOICE_DIR` (varsayılan `./data/voice`), `/voice/*` altında sunulur. mp3, ogg, webm ya da m4a (konteyner magic byte'lardan tanınır), dosya başına 5 MB, toplam `VOICE_QUOTA_MB` (varsayılan 500). S3 yok. API değişmeden ileride S3-uyumlu depoya taşınabilir.
- Deploy: `questdrop` binary'sini içeren Docker imajı (digest-pinli `rust:1.90-slim` build → `debian:bookworm-slim`, root olmayan kullanıcı). `fly.toml` Fly.io'yu hedefler (`fra`, `/ready` kontrolü), ama herhangi bir Docker host olur. **Henüz yayında değil.** `bot` binary'si imajda yok.
- API sözleşmesi: [openapi.yaml](./openapi.yaml) (elle yazıldı, kodu aynalar), RFC 9457 hatalar, swap'te `Idempotency-Key`, cursor sayfalama.
- Varsayılanlar: 10 sn istek zaman aşımı, 2 MB gövde sınırı (Axum varsayılanı, sadece `/v1/voice`'ta kaldırılır), `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, 5 sn Postgres statement timeout, 5 bağlantılık havuz.

## Neden

- Modüler monolit korundu: B ve C, core değişmeden aynı REST + outbox'a takıldı.
- sqlx transaction atomik takası (iki teklif → MATCHED) `SELECT ... FOR UPDATE` ile kapsar.
- `Matcher` trait'i sabit; ileride pgvector tabanlı bir matcher, çağıranlara dokunmadan yerel `EmbeddingMatcher`'ın yerine geçebilir.

## Kapsam Dışı

- MVP'de mikroservis yok, GraphQL yok, S3 kurulumu yok.

## Tedarik zinciri (10. tur denetim)

- Tam lockfile OSV taraması: 1 bulgu, `rsa 0.9.10` (RUSTSEC-2023-0071, Marvin zamanlama saldırısı).
- Hüküm: ERİŞİLEMEZ. Sadece `sqlx-macros-core` (host/build-time) üzerinden gelir;
  üretim hedef grafında yok (`cargo tree`: 0 eşleşme), binary'de `rsa` sembolü yok.
- `sqlx` tarafında `default-features = false`, MySQL/SQLite sürücüleri gemide değil.
- Her bağımlılık değişiminde yeniden tara.
