> 🇬🇧 Click for English: [README.md](./README.md)

# QuestDrop (Rust)

Yarım kalmış yan projeleri takas edilebilir quest'lere çevir. Yoruldun mu? Drop et. Başkasının yarım quest'ini al, yenilik dopaminiyle devam et.

Başarı kriteri: tanımadığın insanların açık havuzda takas yapması.

## Durum (2026-10-12)

| Parça | Durum |
|---|---|
| REST API (`/v1/`), Postgres şeması, auth, ses yükleme | Bitti |
| B: outbox publisher + `bot` binary (log / Discord webhook) | Bitti |
| C: `EmbeddingMatcher` (yerel trigram + kosinüs, `?algo=embedding`) | Bitti |
| Web arayüzü (Askama + HTMX, Türkçe + İngilizce) | Bitti: havuz, giriş, tarayıcıdan ses kaydıyla bırakma, eşleşme, takas, canlandırma, profil, şikâyet |
| Testler | 64 Rust testi yeşil (25 birim + 31 API + 8 web) ve Playwright tarayıcı E2E'si |
| Deploy | **Yayında değil.** `Dockerfile` ve `fly.toml` hazır. |

Web arayüzü [docs/web-ui.tr.md](./docs/web-ui.tr.md)'yi izler; her sayfa JavaScript olmadan çalışır, HTMX sadece hızlandırır.

## Kapsam (MVP = A)

1. Yalın web borsası. Önce A yapılır.
2. B (bot bildirimleri) ve C (daha akıllı eşleşme) sonradan takılır, core değişmez.
3. Her türlü proje: kod, yazı, tasarım. Tip alanı yok, link + ses var.

## Mimari

Tek repo, Rust ile API-first modüler monolit. Mikroservis yok.

```text
[Web UI - Askama+HTMX] -> [API Core - Axum] -> [Postgres+sqlx]
[bot binary] ----------> [outbox_events tablosu] (aynı DB'yi yoklar)
```

- Core dış dünyayı bilmez. Web arayüzü de gelecekteki diğer istemciler de aynı REST'i kullanır (`/v1/`).
- Eşleşme tek trait'tir: `trait Matcher { fn match_items(user, pool) -> Vec<RankedItem> }`. İki uygulaması var: `RuleMatcher` (varsayılan) ve `EmbeddingMatcher`.
- Olaylar bir outbox tablosundan (`outbox_events`) geçer. Bırakma `swap.created`, tamamlanan takas `swap.completed` yazar. Bunları `bot` binary'si iletir.
- Auth: e-posta magic link + HMAC imzalı oturum cookie'si. Dosya: sadece link (S3 yok). Ses: `VOICE_DIR` altında mp3, ogg, webm ya da m4a.

Detay: [docs/design.tr.md](./docs/design.tr.md) · [docs/stack.tr.md](./docs/stack.tr.md) · [docs/plan.tr.md](./docs/plan.tr.md) · [docs/web-ui.tr.md](./docs/web-ui.tr.md) · API sözleşmesi: [docs/openapi.yaml](./docs/openapi.yaml)

English: [design.md](./docs/design.md) · [stack.md](./docs/stack.md) · [plan.md](./docs/plan.md) · [web-ui.md](./docs/web-ui.md)

## API

Login, geçerli bir `qd_session` cookie'si demek. `auth/callback` ve `auth/logout` dışındaki her POST, IP başına dakikada 10 istekle sınırlı.

| Metod | Yol | Login | Not |
|---|---|---|---|
| GET | `/`, `/login`, `/q/{id}` | – | web arayüzü sayfaları (HTML) |
| GET | `/drop`, `/match`, `/take/{offer_id}`, `/revive`, `/me` | ✔ | web arayüzü sayfaları (HTML; oturumsuz → `/login?next=…`) |
| GET | `/health` | – | her zaman 200, `{"status":"ok","db":"up"\|"down"}` |
| GET | `/ready` | – | DB cevap verirse 200, yoksa 503 |
| GET | `/v1/projects?cursor=&limit=` | – | OPEN havuz, yeniden eskiye, cursor sayfalı (varsayılan 20, max 50); öğelerde `offer_id` var |
| GET | `/v1/projects/{id}` | – | tek proje (OPEN ya da MATCHED) |
| POST | `/v1/projects` | ✔ | oturumdaki kullanıcı olarak quest bırak; herhangi bir 24 saatte bırakan başına max 3 |
| POST | `/v1/users/upsert` | ✔ | kendi yetenek profilin (`can_do`, `looking_for`) |
| GET | `/v1/match?algo=` | ✔ | oturumdaki kullanıcı için top-3 + 1 sürpriz, `offer_id` ile; `algo=rule` (varsayılan) veya `embedding` |
| POST | `/v1/swaps` | ✔ | iki teklifin atomik, iki yönlü takası; `Idempotency-Key` opsiyonel |
| GET | `/v1/revive` | ✔ | tamamladığın almalar, freeze paketi + ilk 2 dakika göreviyle |
| POST | `/v1/reports` | ✔ | farklı kullanıcılardan gelen 3 report projeyi kapatır |
| POST | `/v1/auth/request` | – | login linki e-postalar (15 dk tek kullanımlık token) |
| POST | `/v1/auth/callback` | – | token'ı kullanır, oturum cookie'sini kurar (30 gün) |
| POST | `/v1/auth/logout` | – | oturumu siler (idempotent) |
| GET | `/v1/me` | ✔ | oturum kimliği |
| GET | `/v1/me/offers?status=` | ✔ | kendi tekliflerin (takasta ne vereceğini seçmek için); `status` = open, matched ya da closed |
| POST | `/v1/voice` | ✔ | ses yükleme: mp3, ogg, webm ya da m4a (multipart alanı `file`, 5 MB sınır) |
| GET | `/voice/*` | – | kayıtlı ses dosyaları |

Hatalar RFC 9457 problem JSON'u. Bilinmeyen yol JSON 404, yanlış metod JSON 405 döner; sondaki `/` 308 ile yönlenir.

## Hızlı Başlangıç

```bash
cp .env.example .env   # DATABASE_URL'e erişilebilir bir Postgres (13+) yaz
cargo run              # migration'lar açılışta otomatik koşar
```

`http://localhost:3000` aç; `/health` DB durumunu gösterir. Veritabanı yoksa sunucu yine açılır: havuz boş görünür, yazmalar 503 döner.

`SMTP_URL` yoksa login linki gelen kutusu yerine sunucu loguna yazılır (warn seviyesinde, varsayılan filtreyle görünür).

Bot (outbox iletimi): `cargo run --bin bot`.

## Testler

```bash
TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/questdrop_test" cargo test
```

Entegrasyon testleri her tabloyu `TRUNCATE` eder. Veritabanı adresinde `test` geçmiyorsa çalışmayı reddeder.

Tarayıcı E2E'si (Playwright, çalışan bir sunucu ve temiz bir veritabanı ister): bkz. [e2e/README.md](./e2e/README.md).

## Yapılandırma

| Değişken | Varsayılan | Anlamı |
|---|---|---|
| `DATABASE_URL` | – | Postgres adresi; yoksa degrade mod |
| `PORT` | `3000` | dinleme portu |
| `VOICE_DIR` | `./data/voice` | ses notu klasörü |
| `STATIC_DIR` | `./static` | web arayüzünün CSS/JS/HTMX dosyaları |
| `VOICE_QUOTA_MB` | `500` | klasör bu boyuta ulaşınca yükleme reddedilir (507) |
| `SESSION_SECRET` | her açılışta rastgele | en az 16 karakter; production'da şart, yoksa yeniden başlatınca oturumlar düşer |
| `BASE_URL` | `http://localhost:3000` | login linklerinin tabanı; `https://` ise cookie'ye `Secure` eklenir |
| `LOGIN_URL` | `BASE_URL/login` | e-postadaki linkin açtığı sayfa (`?token=` eklenir); arayüzün login sayfasına yönlendir |
| `SMTP_URL` / `MAIL_FROM` | – / `questdrop@localhost` | gerçek e-posta; yoksa log mailer |
| `RUST_LOG` | `questdrop=warn,tower_http=warn` | log filtresi |
| `BOT_SINK` / `BOT_WEBHOOK_URL` | `log` / – | sadece bot: `webhook` Discord webhook'una yollar |
| `PUBLISHER_INTERVAL_SECS` | `5` | sadece bot: yoklama aralığı |
| `TEST_DATABASE_URL` | – | sadece testler |
