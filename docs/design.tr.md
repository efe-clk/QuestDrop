> 🇬🇧 Click for English: [design.md](./design.md)

# QuestDrop — Tasarım Dokümanı (Rust)

Tarih: 2026-09-15 · Kodla son karşılaştırma: 2026-10-12
Durum: Backend yazıldı; web arayüzü henüz yok (bkz. [plan.tr.md](./plan.tr.md))

## 1. Amaç

Sıkılıp yarım bırakmayı utanç olmaktan çıkarıp oyuna çevirmek. Yoruldun mu, projeyi drop et; başkasının yarım quest'ini alıp yenilik dopaminiyle devam et. ADHD'de yarım bırakma döngüsünü kırar.

Başarı kriteri: Tanımadığın insanların açık havuzda takas yapması.

## 2. Kapsam (MVP = A)

- Yalın web borsası. Önce A yapılır.
- B (bot bildirimleri) ve C (daha akıllı eşleşme) sonradan takılır, core değişmez. İkisinin de ilk sürümü var (§8).
- Her türlü proje: kod, yazı, tasarım. Tip alanı yok, link + ses var.

## 3. Mimari

Tek repo, API-first modüler monolit. Mikroservis yok.

```text
[Web UI] -----> [API Core /v1/] -> [Postgres]
[bot binary] -> [outbox_events] (aynı DB'yi yoklar)
```

- Core dış dünyayı bilmez. Her istemci aynı REST'i kullanır.
- Eşleşme tek trait'tir: `trait Matcher { fn match_items(user, pool) -> Vec<RankedItem> }`.
- Olaylar: DB outbox tablosu (`outbox_events`), `bot` binary'si yoklar.
- Auth: e-posta magic link + sunucu tarafı oturum (§4).
- Dosya: sadece link (S3 yok). Ses: mp3; ya `/v1/voice`'a yüklenir ya da `.mp3` ile biten harici bir http(s) adresidir.

## 4. Bileşenler + Veri Modeli

1. **users**: `id, handle (tekil), email (tekil), can_do[], looking_for[]`. Kullanıcı ilk bırakmada ya da ilk girişte oluşur.
2. **projects** (freeze paketi): `title (3-80), one_liner (10-200), link_url, voice_url, skill_needed[] (max 10, her biri 1-32), time_bucket S|M|L, energy LOW|MID|HIGH, status OPEN|MATCHED|CLOSED, giver_id, embedding JSONB (kullanılmayan yer tutucu)`.
   - `time_bucket` anlamı: S = 2 saatten az, M = 2-8 saat, L = 8 saatten fazla.
3. **swap_offers**: `project_id, giver_id, status OPEN|MATCHED|CLOSED`. Her bırakma tam bir teklif oluşturur. **Takaslar proje id'siyle değil, teklif (offer) id'siyle yapılır.**
4. **matches**: `taker_id, given_id, taken_id (proje id'leri), score, reason`.
5. **reports**: `project_id, reporter_id, reason (10-300)`; aynı kişi bir projeye bir kez.
6. **magic_tokens** (sadece SHA-256 özeti, 15 dk, tek kullanım) ve **sessions** (30 gün).
7. **idempotency_keys**: `(key, taker)` başına saklanan takas cevapları.

Kapsam dışı: yorum, like, DM, puan, kategori ağacı.

### Kimlik kuralları

- Bırakma `handle` + `email` bilgisini gövdede taşır, login istemez. Var olan bir e-posta kendi handle'ıyla gelmezse 409 döner. Yeni bir çift kullanıcı oluşturur.
- Giriş: `POST /v1/auth/request` bir token saklar ve `BASE_URL/login?token=…` gönderir. `/login`'deki bir istemci sayfası bu token'ı `/v1/auth/callback`'e POST etmelidir; callback `qd_session` cookie'sini kurar (`HttpOnly; SameSite=Lax`, `BASE_URL` https ise `Secure`). **`/login` sayfası henüz yok.**
- Takas, report, profil düzenleme ve ses yükleme oturum ister; gövdedeki id'ler oturum sahibiyle aynı olmalıdır (yoksa 403).

## 5. Veri Akışı

1. **Bırak**: form → validasyon (§4) → `voice_url` bir `/voice/…` yoluysa dosyanın var olması gerekir.
2. **Dondur**: tek transaction proje (OPEN), teklifi (OPEN) ve bir outbox `swap.created` olayını yazar. Bırakan başına herhangi bir 24 saatte max 3 (429 + `Retry-After`).
3. **Eşleş**: `GET /v1/match?user_id=` kullanıcının kendisine ait olmayan en yeni en fazla 100 OPEN projeyi sıralar, top-3 + 1 sürpriz döner.
   - `RuleMatcher` (varsayılan): skor = 0,8 × yetenek uyumu + 0,2 × rastgele. Yetenek uyumu, `skill_needed` ile kullanıcının `can_do` ve `looking_for` listelerinin örtüşmesinin ortalamasıdır. **`time_bucket` ve `energy` henüz skora girmiyor.**
   - `EmbeddingMatcher` (`algo=embedding`): aynı yetenek kelimeleri üzerinde deterministik trigram-hash kosinüs; en iyi 4'ü döner, rastgele terim yok.
4. **Takas**: **iki yönlü bir değiş tokuş**. Alan kişi kendi OPEN tekliflerinden birini verir, başkasının OPEN teklifini alır. Tek transaction iki teklifi (id sırasıyla) kilitler, iki teklifi ve iki projeyi MATCHED yapar, deterministik yetenek uyumunu `score` olarak içeren bir `matches` satırı ve bir outbox `swap.completed` olayı yazar. Hiç bırakmamış biri quest alamaz.
5. **Canlandır**: `GET /v1/revive?user_id=` kullanıcının aldığı her projeyi yeniden eskiye listeler; her birinde freeze paketi ve sabit 4 adımlı "ilk 2 dakika" görevi vardır.

## 6. Hata / Moderasyon

- Ses/link eksik ya da geçersizse kayıt yok; cevap bozulan her kuralı listeler.
- Kendinle takas ve kendi projeni almak reddedilir (409).
- Günde max 3 bırakma. Farklı kişilerden 3 report, projeyi ve açık tekliflerini CLOSED yapar (oto-gizle). İnceleme kuyruğu reports tablosu; inceleme arayüzü yok. Silme yok, kapatma var.
- AI'dan kural-tabanlıya düşme gerekmiyor: `EmbeddingMatcher` yerel ve deterministik, varsayılan zaten `rule`.
- Bot çökerse core ayakta kalır; olaylar outbox'ta bekler.

### Bilinen sınırlar (2026-10-12)

- Ses süresi (10-60 sn olması planlanmıştı) **denetlenmiyor**. Sadece biçim (magic bytes / `.mp3`) ve boyut (5 MB) denetleniyor.
- `/v1/match` ve `/v1/revive` login olmadan herhangi bir `user_id` kabul eder; bir kullanıcının id'sini bilen herkes onun eşleşmelerini ve aldığı projeleri görebilir.
- Bırakma login istemez; bir kullanıcının handle + e-postasını bilen onun adına bırakma yapabilir.
- Rate limiter bellekte ve süreç başınadır (yeniden başlatınca sıfırlanır, birden çok instance arasında paylaşılmaz). `X-Forwarded-For`'un en soldaki değerine güvenir.
- `swap.created` olay adı takas için değil, **bırakma** için üretiliyor. Dış tüketiciler buna bağlanmadan önce adı düzeltilmeli.

## 7. Test

- Birim (16): validasyon, matcher sırası ve aralığı, auth yardımcıları, ilk görev, Discord gövdesi.
- Entegrasyon (29, gerçek Postgres): bırakma/havuz/sınır, takas atomikliği ve yarışlar (aynı anahtarla 5 paralel istek 1×201 + 4×200 verir), idempotency, report ve oto-gizleme, auth ve cookie, ses yükleme, hata biçimleri.
- Tarayıcıdan E2E: web arayüzü olmadan mümkün değil. Yük testi yok.

## 8. B/C Genişleme Kapıları

- **B (bitti, v1):** `bot` binary'si `outbox_events`'i yoklar (`FOR UPDATE SKIP LOCKED`, en az bir kez teslim) ve `LogSink` ya da `WebhookSink`'e (Discord incoming webhook) iletir. Telegram sink'i yok.
- **C (bitti, v1):** `EmbeddingMatcher` aynı trait arkasında. `embedding` kolonu hâlâ kullanılmıyor; pgvector tabanlı bir matcher, çağıranlara dokunmadan bunun yerine geçebilir.
