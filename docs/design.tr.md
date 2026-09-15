> 🇬🇧 Click for English: [design.md](./design.md)

# QuestDrop — Tasarım Dokümanı

Tarih: 2026-09-15
Durum: Taslak, onay bekliyor

## 1. Amaç

Sıkılıp yarım bırakmayı utanç olmaktan çıkarıp oyuna çevirmek. Yoruldun mu, projeyi drop et; başkasının yarım quest'ini alıp yenilik dopaminiyle devam et. ADHD'de yarım bırakma döngüsünü kırar.

Başarı kriteri: Tanımadığın insanların açık havuzda takas yapması.

## 2. Kapsam (MVP = A)

- Yalın web borsası. Önce A yapılır.
- B (Discord/Telegram botu) ve C (AI eşleşme) sonradan takılır, core değişmez.
- Her türlü proje: kod, yazı, tasarım. Tip alanı yok, link + ses var.

## 3. Mimari

Tek repo, API-first modüler monolit. Mikroservis yok.

```text
[Web UI] -> [API Core] -> [DB]
[Bot] -----> [API Core] (ileride, aynı API)
```

- Core dış dünyayı bilmez. Web de bot da aynı REST'i kullanır.
- Eşleşme tek interface: `Matcher.match(user, pool) -> ranked_list`. Şimdi kural-tabanlı, sonra `EmbeddingMatcher` aynı interface ile takılır.
- Event: `swap.created` için basit DB outbox tablosu. Bot bildirimi ve AI yeniden-sıralama bunu dinler.
- Auth: magic link. Dosya: link (S3 yok). Ses: 30sn mp3.

## 4. Bileşenler + Veri Modeli

1. Identity: id, handle, email, yapabildiklerim[], aradıklarım[]
2. Project (Freeze Paketi): title, one_liner, link_url, voice_url (10-60sn zorunlu), skill_needed[], time_bucket [S=<2saat / M=2-8saat / L=8saat+], energy [düşük/orta/yüksek], status, embedding NULL
3. Pool/Swap: swap_offers (project_id, giver_id, status open/matched/closed) + matches (taker_id, verilen, alınan, score, neden)
4. Adaptör: Web şimdi, Bot sonra. Ek tablo yok.

Yok: yorum, like, DM, puan, kategori ağacı. Moderasyon için sadece reports (project_id, neden).

## 5. Veri Akışı

1. Bırak: 30sn form, validation zorunlu.
2. Dondur: projects (open) + swap_offers (open).
3. Eşleş: top-3 + 1 sürpriz, kullanıcı seçer. Skor = %80 uyum + %20 rastgele.
4. Takas: tek transaction, iki teklif matched, row-lock ile çift kapma engellenir.
5. Canlandır: alıcıya freeze paketi + ilk 2 dakika görevi.

## 6. Hata / Moderasyon

- Ses/link yoksa kaydetme, nedenini söyle. Kendinle eşleşme yasak.
- Adaptör çökerse core ayakta. AI matcher hata verirse kural-tabanlıya düş.
- Günde max 3 bırakma. 3 report = oto-gizle + inceleme kuyruğu. Silme yok, kapatma var.

## 7. Test

- Unit: matcher skoru.
- Integration: takas atomikliği, API+DB.
- E2E: bırak -> eşleş -> al. Yük testi yok.

## 8. B/C Genişleme Kapıları

- B: yeni Bot adaptörü, aynı endpointler, event dinler.
- C: embedding alanı hazır, Matcher interface sabit, veri birikince takılır.
