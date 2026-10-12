> 🇬🇧 Click for English: [web-ui.md](./web-ui.md)

# QuestDrop — Web Arayüzü Tasarımı

Tarih: 2026-10-12 · Kapsam: [plan.tr.md](./plan.tr.md) Faz 5 · API: [openapi.yaml](./openapi.yaml)

Bu doküman QuestDrop'un web arayüzünün nasıl olması gerektiğini anlatır. Arayüzü kim yazarsa yazsın bu dokümanı sözleşme olarak kullanabilir. Her ekran, bugün var olan API'ye bağlıdır. API'de eksik olan yerler §10'da ayrıca listelenmiştir.

> **Durum (2026-10-12): uygulandı.** Kod: `src/web.rs`, `templates/`, `static/`, `i18n/`. §13'teki açık sorular önerildiği gibi kapatıldı: bırakma login ister (§5.2), ses için A seçeneği (§6), İngilizce + Türkçe (§9); harici ses linkleri "harici ses" etiketiyle açık kaldı. Sapmalar: kaydedicinin yüklemeden önce kaydı dinletebilmesi için CSP `media-src` `blob:`'a da izin verir; masaüstü menü 768 px yerine 960 px altında alt sekme çubuğuna geçer, çünkü arada dört etiket ve kullanıcı adı sığmıyor.

---

## 1. Kimin için, ne hissettirmeli?

Hedef kullanıcı yarım bıraktığı projelerden utanan, dikkati çabuk dağılan (ADHD'li ya da öyle hisseden) biri. Arayüzün her kararı bundan türer:

| İlke | Arayüzdeki karşılığı |
|---|---|
| **Utanç yok, oyun var** | "Bıraktın" değil "Quest'i havuza attın". Kapatılmış proje yok, "başkasının elinde devam ediyor" var. Kırmızı hata ekranları yerine sakin, yönlendirici mesajlar. |
| **30 saniye kuralı** | Bırakma formu 30 saniyede doldurulabilmeli. Zorunlu alanlar: başlık, tek cümle, link, ses, süre, enerji. Yetenekler isteğe bağlı. |
| **Tek ekran, tek ana eylem** | Her sayfada tek bir büyük birincil buton. İkincil eylemler küçük ve geride. |
| **Az seçenek** | Eşleşme ekranı 4 kart gösterir (3 + 1 sürpriz), sonsuz liste değil. Seçim felci yok. |
| **Yenilik dopamini** | Sürpriz kartı görsel olarak ayrışır. Takas anı küçük bir kutlama ister (hafif animasyon, `prefers-reduced-motion`'a saygılı). |
| **Hemen başlama** | Takastan sonra kullanıcı doğrudan "ilk 2 dakika" görevine düşer. Ayrı bir sayfaya gitmeye gerek kalmaz. |
| **Sakin** | Bildirim yağmuru, rozet, puan, sayaç yok (tasarım dokümanı bunları bilerek dışarıda bıraktı). |

Başarı ölçüsü tasarım dokümanındakiyle aynı: **tanımadığın insanların açık havuzda takas yapması**. Arayüz bunu, siteyi ilk kez açan birinin birkaç dakikada ilk quest'ini bırakabilmesiyle mümkün kılar.

---

## 2. Teknik yaklaşım

**Önerilen: aynı Rust binary'sinde server-render (Askama + HTMX).** Stack dokümanında da böyle kilitlenmiş.

- Sayfalar Askama şablonlarıyla sunucuda üretilir. HTMX sadece parça güncellemeleri için kullanılır: "daha fazla yükle", form hatasını yerinde gösterme, takas onayı.
- **Her form JavaScript olmadan da çalışır.** HTMX yoksa normal form POST'u ve sayfa yenilemesi olur. HTMX bunu sadece hızlandırır. Tek istisna tarayıcıdan ses kaydıdır (§6); onun da dosya yükleme yedeği vardır.
- Arayüz route'ları (`/`, `/drop`, `/match`…) veritabanına doğrudan `db::` fonksiyonlarıyla erişir; kendi REST API'sini HTTP üzerinden çağırmaz. REST `/v1/` bot ve dış istemciler için kalır. Doğrulama (`validate.rs`), kurallar ve transaction'lar tek yerde kalır.
- Aynı origin olduğu için oturum cookie'si (`qd_session`, `SameSite=Lax`) kendiliğinden çalışır. **CORS gerekmez.**
- Build adımı yok. Node, npm ya da bundler yok. `static/` klasöründe tek bir CSS dosyası, HTMX'in sabitlenmiş bir kopyası (`htmx.min.js`, yaklaşık 15 KB) ve ses kaydı için küçük bir JS dosyası bulunur.
- Hedef: her sayfa 50 KB'ın altında (ses hariç), yavaş 3G'de 2 saniyede kullanılabilir.

**Alternatif (önerilmez):** ayrı bir SPA (React/Svelte vb.) ayrı bir origin'de. Bu yolu seçersen CORS (credentials ile), CSRF ve cookie domain ayarları gerekir (bkz. §10, madde 4). Ayrıca stack dokümanındaki "Node yok" kararı bozulur.

---

## 3. Sayfa haritası

| URL | Sayfa | Giriş | Ana eylem |
|---|---|---|---|
| `/` | Havuz | – | "Quest bırak" |
| `/login` | E-posta ile giriş | – | "Link gönder" |
| `/login?token=…` | Giriş tamamlama | – | "Giriş yap" |
| `/drop` | Bırak | ✔ (§5.2) | "Havuza at" |
| `/match` | Sana uygun 4 quest | ✔ | "Bunu al" |
| `/take/{offer_id}` | Takas onayı | ✔ | "Takas et" |
| `/revive` | Aldığım quest'ler | ✔ | "İlk 2 dakikaya başla" |
| `/me` | Profil: yetenekler, açık quest'lerim, çıkış | ✔ | "Kaydet" |
| `/q/{project_id}` | Tek quest detayı (paylaşılabilir) | – | "Bunu al" |

Giriş gerektiren bir sayfaya oturumsuz gelen kullanıcı `/login?next=/match` adresine yönlendirilir. Giriş bitince `next` sayfasına geri döner. `next` sadece site içi yol olabilir; açık yönlendirmeye izin verilmez.

**Üst menü** (her sayfada, mobilde alt sekme çubuğu): Havuz · Bırak · Eşleş · Quest'lerim. Sağda giriş yapmışsa `@handle`, yapmamışsa "Giriş".

---

## 4. Ana akışlar

### 4.1 İlk ziyaret → ilk bırakma
1. `/` havuzu görür: üstte tek cümlelik açıklama ve "Quest bırak" butonu.
2. "Quest bırak" → oturum yoksa `/login?next=/drop`: e-posta + isteğe bağlı handle → "Link gönder".
3. E-postadaki link → `/login?token=…` → "Giriş yap" → `/drop`. Handle ve e-posta oturumdan gelir, formda görünmez.
4. Başlık, tek cümle, link, ses, yetenekler, süre, enerji → "Havuza at".
5. Onay ekranı: "Quest'in havuzda." + "Bana uygun olanlara bak" (`/match`).

Neden ilk eylem bırakmak? Takas iki yönlüdür: almak için kendi açık quest'ini vermek gerekir (design §5.4). Hiç bırakmamış biri alamaz.

### 4.2 Giriş
1. `/login`: e-posta (+ ilk kez geliyorsa isteğe bağlı handle). "Link gönder" → `POST /v1/auth/request` karşılığı.
2. Ekranda "Gelen kutunu kontrol et, link 15 dakika geçerli." Aynı e-postayla tekrar gönderme 60 saniye sonra açılır.
3. Linke tıklanınca `/login?token=…` açılır.
   - **Token GET isteğinde tüketilmemeli.** E-posta istemcilerinin link önizleyicileri token'ı yakmasın diye sayfa önce bir "Giriş yap" butonu gösterir. Buton token'ı POST eder; POST başarılı olursa cookie kurulur ve `next` sayfasına ya da `/match`'e yönlendirilir.
4. Token geçersiz ya da süresi dolmuşsa: "Bu linkin süresi dolmuş. Yenisini gönderelim mi?" ve e-posta alanı.

### 4.3 Eşleş → al → canlandır (çekirdek döngü)
1. `/match`: 4 kart. Kullanıcının yeteneği yoksa üstte "Yeteneklerini eklersen eşleşmeler isabetli olur → /me" bandı görünür (yetenek yoksa eşleştirici tamamen rastgele sıralar).
2. "Bunu al" → `/take/{offer_id}`: üstte alınacak quest, altında kullanıcının **açık quest'leri** (radyo listesi). Tek açık quest'i varsa seçili gelir.
3. "Takas et" → swap (Idempotency-Key ile, §7.3) → kutlama → doğrudan `/revive#{match_id}`.
4. `/revive`: aldığı quest kartı; ses oynatıcı, link ve 4 adımlı "ilk 2 dakika" görevi (işaretlenebilir kutular, sadece tarayıcıda tutulur). İsteğe bağlı 2 dakikalık geri sayım.

Hiç açık quest'i yoksa `/take` ekranı takas yerine "Önce bir quest bırak" der ve `/drop?next=/take/{offer_id}`'e yönlendirir.

---

## 5. Ekranlar

Her ekran için durumlar aynı sırayla düşünülmeli: **dolu · boş · yükleniyor · hata · veritabanı yok (503)**.

### 5.1 Havuz (`/`)
- **Üst bölüm:** "Yoruldun mu? Bırak. Başkasının yarım quest'ini al." + "Quest bırak" (birincil) + "Bana uygun olanlar" (ikincil, `/match`).
- **Liste:** `QuestCard` ızgarası (mobilde tek sütun, masaüstünde 2–3 sütun). Kaynak: `db::list_pool`, sayfa başı 20.
- **Daha fazla:** listenin altında "Daha fazla göster". HTMX `hx-get="/?cursor={next_cursor}" hx-target="#pool" hx-swap="beforeend"`. `next_cursor` yoksa buton kaybolur.
- **Filtreler (v1'de yok, v2):** süre (S/M/L) ve enerji. API'de filtre olmadığı için şimdilik eklenmez.
- **Boş durum:** "Havuz boş. İlk quest'i sen bırak." + buton.
- **503:** sayfanın üstünde sakin bir bant: "Şu an havuza ulaşamıyoruz, birazdan tekrar dene." Liste yerine boş durum gösterilir. Sunucu degrade modda zaten boş havuz döner.

### 5.2 Bırak (`/drop`)
**Karar: bırakma login ister.** Bugün API girişsiz bırakmaya izin veriyor; ama ses yükleme (`/v1/voice`) zaten login istiyor ve girişsiz bırakma başkası adına bırakmaya açık (design §6). Login'den sonra form 30 saniye kuralına uyar. Bu karar §10, madde 8'deki backend değişikliğini gerektirir.

Tek sütunlu form. Alan sırası ve kuralları `validate.rs`'deki kurallarla birebir aynı olmalı:

| Alan | Kontrol | Kural (sunucu) | Yardım metni |
|---|---|---|---|
| Başlık | text | 3–80 karakter | "Quest'in adı" |
| Tek cümle | text | 10–200 karakter | "Nerede kaldın?" |
| Link | url | http(s), ≤2048 | "Repo, doküman, Figma…" |
| Ses notu | kaydedici + dosya yükleme | §6 | "10–60 saniye: ne yapmak istedin, nerede takıldın?" |
| Yetenekler | chip girişi | ≤10, her biri 1–32, küçük harfe çevrilir | "rust, illüstrasyon, metin yazarlığı…" |
| Süre | 3'lü segment | S / M / L | S: "2 saatten az" · M: "2–8 saat" · L: "8 saatten fazla" |
| Enerji | 3'lü segment | LOW / MID / HIGH | "Düşük / Orta / Yüksek enerji ister" |

Handle ve e-posta formda yok; sunucu oturumdan doldurur.

- Süre ve enerji için varsayılan seçili değer **yok**; bilinçli seçim istenir. İki segment büyük ve dokunmaya uygun.
- **Hatalar:** sunucu `errors[]` listesi döner (her bozuk kural bir satır). Arayüz her satırı ilgili alanın altına yazar; eşlenemeyenleri formun üstündeki özet kutusuna koyar. Form verisi asla kaybolmaz.
- **Günlük sınır (429 + Retry-After):** "Bugünlük 3 quest'ini bıraktın. Bir sonrakini {saat} sonra bırakabilirsin." Gönder butonu pasif olur.
- **Başarı:** "Quest'in havuzda 🎯" + quest kartının önizlemesi + "Bana uygun olanlara bak" (`/match`). `next` varsa (ör. `/take/…`'den gelindiyse) oraya dönülür.

### 5.3 Giriş (`/login`)
Bkz. §4.2. Tek alan, tek buton. `auth/request` her zaman "gönderildi" der (hesabın var olup olmadığını sızdırmaz); arayüz de aynı mesajı gösterir. Geliştirme ortamında (SMTP yok) link loga düşer; o durumda sayfanın altında küçük bir "geliştirici modu: link sunucu logunda" notu gösterilebilir (sadece `BASE_URL` localhost ise).

### 5.4 Eşleş (`/match`)
- 4 kart: ilk 3'ü "Sana uygun", 4.'sü **"Sürpriz"** etiketli ve farklı renkte çerçeveli.
- Her kartta uyumun nedeni insan diliyle yazar. API `reason` alanında `fit=0.50 rand=0.31` döndürür; arayüz bunu ham göstermez, "Yeteneklerinle %50 örtüşüyor" diye yazar. Sürpriz kartında "Tamamen yeni bir şey" yazar.
- "Yenile" butonu: kural tabanlı eşleştiricide %20 rastgelelik olduğu için her yenilemede sıra biraz değişir.
- `?algo=embedding` v1'de arayüzde gösterilmez.
- **Boş:** "Şu an havuzda sana uygun quest yok" + "Havuzun tamamına bak".

### 5.5 Takas onayı (`/take/{offer_id}`)
- Üstte alınacak quest (tam kart, ses oynatıcı açık), altında "Karşılığında hangi quest'ini veriyorsun?" ve kullanıcının açık quest'leri.
- Açıklama satırı: "Takas iki yönlüdür: senin quest'in de karşı tarafa geçer."
- "Takas et" butonu tıklanınca kilitlenir. İstek Idempotency-Key taşır (§7.3).
- **409 durumları** (sunucu `detail`'ine göre):
  - "offer is no longer open" → "Biri senden önce aldı. Başka bir quest'e bak." + `/match`
  - "give offer is not yours" / "cannot take your own project" → normal akışta olmaz; olursa genel hata.
- **Başarı:** kısa kutlama → `/revive#{match_id}`.

### 5.6 Quest'lerim / Canlandır (`/revive`)
- **Sekme 1 – Aldıklarım** (`all_revives` verisi): en yenisi üstte ve açık. Kartta freeze paketi (başlık, tek cümle, link, ses, yetenekler), bırakanın handle'ı ve **İlk 2 dakika** kutusu:
  1. Linki aç, 30 saniye göz gezdir
  2. Ses notunu baştan sona dinle
  3. En küçük `{yetenek}` parçasını yerelde çalıştır
  4. Sıradaki tek adımı yaz
  - Kutular işaretlenebilir; durum `localStorage`'da `questdrop:revive:{match_id}` anahtarıyla tutulur (sunucuya gitmez).
  - "2 dakikayı başlat" butonu görünür bir geri sayım açar. Bitince "Devam etmek istersen artık senin quest'in." yazar.
- **Sekme 2 – Bıraktıklarım:** açık ve takas edilmiş quest'ler. **API'de yok** (§10, madde 3).
- **Boş:** "Henüz quest almadın. Sana uygun olanlara bak."

### 5.7 Profil (`/me`)
- Handle ve e-posta (salt okunur; değiştirme v1'de yok).
- **Yapabildiklerim** ve **Aradıklarım**: chip girişi, her biri en fazla 20. Kaydet → `upsert_profile`.
- Açık quest'lerim listesi (§10, madde 3).
- "Çıkış yap" → logout → `/`.

### 5.8 Quest detayı (`/q/{project_id}`)
Paylaşılabilir tek kart sayfası. Open Graph etiketleri (başlık, tek cümle) olur. Quest artık açık değilse "Bu quest yeni bir sahibine geçti" yazar ve havuz linki verir. **API'de tekil proje getiren endpoint yok**; `db` katmanına küçük bir sorgu eklenir (§10, madde 6).

### 5.9 Şikâyet
Her `QuestCard`'da "⋯" menüsü → "Şikâyet et" → küçük bir dialog (sebep, 10–300 karakter). Login ister. 409 "already reported" → "Bunu zaten bildirdin, teşekkürler." Kendi quest'inde menüde bu seçenek görünmez. Cevaptaki `hidden: true` ise kart listeden yumuşakça kaybolur.

---

## 6. Ses notu (en riskli parça)

**Durum (2026-10-12): A seçeneğiyle çözüldü.** Backend artık mp3, ogg, webm ve m4a kabul ediyor. Bağlam için asıl sorun: backend sadece **mp3** kabul ediyordu (magic byte + `.mp3` uzantısı). Tarayıcılar ise yerleşik kayıtta mp3 üretmiyor: Chrome/Firefox Opus'u webm/ogg içinde, Safari AAC'yi mp4 içinde kaydediyor ([MDN `isTypeSupported`](https://developer.mozilla.org/en-US/docs/Web/API/MediaRecorder/isTypeSupported_static), [addpipe](https://blog.addpipe.com/recording-audio-in-the-browser-using-pure-html5-and-minimal-javascript)). Tarayıcıdan kayıt yapılıp doğrudan yüklenince sunucu 400 dönüyordu.

Seçenekler:

| | Yaklaşım | Artı | Eksi |
|---|---|---|---|
| A | **Backend Opus/AAC'yi de kabul etsin** (webm, ogg, m4a magic byte'ları) | Yerleşik kayıt, küçük dosya, JS az | `validate.rs` + `looks_like_mp3` + testler değişir; `voice_url` uzantı kuralı genişler |
| B | Tarayıcıda mp3'e çevir (lamejs benzeri kütüphane) | Backend değişmez | ~150 KB ek JS, mobilde yavaş, bakımı zayıf kütüphaneler |
| C | Sunucuda ffmpeg ile mp3'e çevir | İstemci basit | İmaj büyür, CPU, yeni saldırı yüzeyi |

**Seçilen: A.** Arayüz `MediaRecorder.isTypeSupported()` ile `audio/webm;codecs=opus` → `audio/ogg;codecs=opus` → `audio/mp4` sırasıyla dener. Dosya yükleme seçeneği her zaman açık kalır (hazır ses dosyası olanlar için).

**Kaydedici bileşeni:**
- Büyük yuvarlak "Kaydet" butonu, kayıt sırasında süre sayacı ve ses seviyesi çubuğu.
- **10 saniyeden kısa kayıt kabul edilmez**, 60. saniyede kayıt otomatik durur. Sunucu süreyi denetlemediği için bu kural sadece arayüzde yaşar (design §6).
- Kayıttan sonra: dinle · yeniden kaydet · kullan. "Kullan" dosyayı hemen `/v1/voice`'a yükler, yükleme çubuğu gösterir ve dönen `voice_url`'i gizli alana yazar. Form gönderilmeden önce ses yüklemesi bitmiş olmalı.
- Mikrofon izni reddedilirse: "Mikrofon izni yok. Hazır bir ses dosyası yükleyebilirsin."
- 413 → "Dosya 5 MB'tan büyük." · 507 → "Ses depomuz doldu, bize haber ver." · 401 → oturum düşmüş, girişe yönlendir (form verisi `sessionStorage`'da saklanır).

**Oynatıcı:** yerleşik `<audio controls preload="none">`. Harici `voice_url` (başka domain) çalınırken kullanıcının IP'si o domaine gider. v1'de harici ses linkleri görünür bir "harici ses" etiketiyle gösterilir. Uzun vadede sadece `/voice/` kabul edilmesi önerilir.

---

## 7. Hata ve durum yönetimi

### 7.1 HTTP → arayüz eşlemesi

| Kod | Ne zaman | Arayüz |
|---|---|---|
| 400 + `errors[]` | validasyon | alan altı mesajlar + özet kutusu |
| 401 | oturum yok/düştü | `/login?next=…`; doldurulmuş form `sessionStorage`'da kalır |
| 403 | id oturumla uyuşmuyor | genel hata (normal akışta olmamalı) |
| 404 | bulunamadı | sayfa içi boş durum |
| 409 | takas yarışı, tekrar şikâyet, handle çakışması | sunucu `detail`'ine göre dostça metin (§5.5, §5.9) |
| 413 / 507 | ses | §6 |
| 429 | IP limiti ya da günlük bırakma sınırı | `Retry-After` ile geri sayım, buton pasif |
| 503 | veritabanı yok | sayfa üstünde bant, okuma ekranları boş durum, yazma butonları pasif |
| 5xx diğer | – | "Bir şeyler ters gitti, tekrar dene" + tekrar dene butonu |

Sunucu mesajları İngilizce gelir; arayüz `detail` metnini **eşleme tablosuyla** çevirir, eşleşmeyeni genel mesaja düşürür. Ham sunucu metni son kullanıcıya gösterilmez.

### 7.2 HTMX hata davranışı
HTMX 4xx/5xx cevaplarını varsayılan olarak sayfaya basmaz. Arayüz handler'ları hata durumunda **200 + hata parçası** (form yeniden, hatalarla) döndürür ya da `HX-Retarget`/`HX-Reswap` başlıkları kullanır. Ekran okuyucular için hata kutusu `role="alert"` taşır.

### 7.3 Çift gönderim
- Tüm gönder butonları istek sırasında `disabled` olur (`hx-disabled-elt`).
- Takas formu sayfa render edilirken üretilen bir UUID'yi gizli alanda taşır ve onu `Idempotency-Key` olarak kullanır. Form tekrar gönderilirse sunucu aynı sonucu 200 ile tekrarlar (`Idempotent-Replayed: true`); arayüz bunu başarı sayar.

---

## 8. Görsel dil

- **Ton:** sakin, sıcak, hafif oyunsu. Bir "quest panosu", bir dashboard değil.
- **Renk:** nötr açık zemin + tek vurgu rengi (birincil butonlar) + sürpriz kartı için ikinci bir vurgu. Kırmızı sadece gerçek hatalarda. Tüm renkler CSS değişkeni; `prefers-color-scheme: dark` ile koyu tema.
- **Tipografi:** sistem fontları (`system-ui`), gövde 16–18 px, satır yüksekliği 1.5. Başlıklar kalın, kısa.
- **Simge dili:** süre S/M/L için küçük/orta/büyük kum saati; enerji LOW/MID/HIGH için 1/2/3 dolu pil. **Simge yanında her zaman metin** olur, simge tek başına anlam taşımaz.
- **`QuestCard`:** başlık · tek cümle · yetenek chip'leri · süre + enerji rozetleri · ses oynatıcı · "linke git" · `@bırakan` · birincil eylem. Kartın tamamı değil, sadece eylem butonu tıklanabilir.
- **Hareket:** takas anında 600 ms'lik kutlama, sayfa geçişlerinde yok. `prefers-reduced-motion` ise tamamen kapalı.
- **Boşluk:** 8 px'lik ızgara; dokunma hedefleri en az 44×44 px.

---

## 9. Erişilebilirlik, dil, güvenlik

**Erişilebilirlik (WCAG 2.2 AA):**
- Klavyeyle tam kullanım; görünür odak halkası; "içeriğe geç" linki.
- HTMX ile değişen bölgeler `aria-live="polite"`. Hata özetleri `role="alert"`.
- Segment kontrolleri (süre/enerji) gerçek `<input type="radio">` + `<fieldset><legend>`.
- Ses kaydedici: butonun durumu (`aria-pressed`) ve kayıt süresi ekran okuyucuya duyurulur.
- Ses notunun metin karşılığı yok. Tek cümle alanı bu açığı kısmen kapatır; v2'de isteğe bağlı "ses notunun özeti" alanı düşünülebilir.
- Kontrast en az 4.5:1; 200% zoom'da yatay kaydırma yok.

**Dil:** v1 İngilizce + Türkçe. Metinler şablonlarda değil, tek bir sözlük dosyasında (`i18n/en.toml`, `i18n/tr.toml`). Dil seçimi `Accept-Language`'a göre yapılır, kullanıcı değiştirebilir (cookie). Kullanıcının girdiği içerik çevrilmez.

**Güvenlik:**
- Askama otomatik kaçışlıdır; `|safe` filtresi kullanıcı içeriğinde **asla** kullanılmaz.
- `link_url` her zaman `rel="noopener noreferrer nofollow ugc" target="_blank"` ile gösterilir. Sunucu zaten sadece http(s) kabul ediyor.
- **CSRF:** arayüz form handler'ları (`POST /drop`, `/take`, `/me`, `/logout`) `Origin`/`Sec-Fetch-Site` başlığını kontrol eder; `SameSite=Lax` tek başına yeterli sayılmaz. Alternatif olarak oturuma bağlı bir CSRF token'ı kullanılır.
- **CSP:** `default-src 'self'; media-src 'self' https:; img-src 'self' data:; script-src 'self'; style-src 'self'; frame-ancestors 'none'`. Inline script yok (HTMX `hx-*` öznitelikleri CSP ile uyumlu).
- `next` parametresi sadece `/` ile başlayan ve `//` ile başlamayan yolları kabul eder.

---

## 10. Arayüzden önce API/backend'de yapılması gerekenler

| # | İş | Neden | Büyüklük |
|---|---|---|---|
| 1 | ✅ Backend kısmı bitti: `LOGIN_URL`'i arayüzün login sayfasına ayarla. Sayfanın kendisi (GET token'ı gösterir, POST tüketir) arayüz işi | e-postadaki link onu açmalı | küçük |
| 2 | ✅ Havuz ve eşleşme öğelerinde `offer_id` | takas teklif id'siyle yapılıyor | bitti |
| 3 | ✅ `GET /v1/me/offers?status=` | `/take` ve `/revive` sekme 2 için şart | bitti |
| 4 | (Sadece ayrı origin SPA seçilirse) credentials'lı CORS | aynı binary'de gerekmez | küçük |
| 5 | ✅ Ses formatı: mp3, ogg, webm, m4a (§6, seçenek A) | tarayıcıdan kayıt | bitti |
| 6 | Tekil quest sorgusu (`/q/{id}` için) | paylaşılabilir sayfa | küçük |
| 7 | `/match` ve `/revive` arayüzde oturumdan gelen kullanıcıyla çağrılır; REST tarafı login'e bağlanır | design §6'daki gizlilik açığı | küçük |
| 8 | Bırakmayı login'e bağla (§5.2 kararı) | başkası adına bırakma açığı | küçük; testler güncellenir |
| 9 | Ana sayfa şablonundaki `/v1/match` linkini kaldır | `user_id`'siz 400 veriyor | çok küçük |

1, 2, 3 ve 5 bittiği için (2026-10-12) arayüz yazılınca çekirdek döngü (bırak → eşleş → al) mümkün. §5.2 kararı için madde 8 hâlâ gerekli.

---

## 11. Uygulama sırası (dilimler)

Her dilim kendi başına yayınlanabilir:

1. **İskelet:** `base.html` (menü, tema, i18n), `static/` (CSS, htmx), CSP ve güvenlik başlıkları. Havuz sayfası (salt okunur, sayfalama).
2. **Giriş:** `/login`, `/login?token=`, çıkış, `next` yönlendirmesi, oturum yardımcıları. (§10: 1)
3. **Bırak:** form, kaydedici, yükleme, hata eşleme, günlük sınır. (§10: 5, 8)
4. **Eşleş + al:** `/match`, `/take/{offer_id}`, Idempotency-Key, kutlama. (§10: 2, 3)
5. **Canlandır + profil + şikâyet:** `/revive`, ilk 2 dakika, `/me`, şikâyet dialog'u. (§10: 7)
6. **Cila:** `/q/{id}` + Open Graph, koyu tema, boş durumlar, mikro metinler, erişilebilirlik denetimi. (§10: 6, 9)

---

## 12. Test

- **E2E (Playwright, gerçek Postgres):** iki tarayıcı bağlamı ile iki kullanıcı: A ve B giriş yapar (token log mailer'dan okunur) ve birer quest bırakır, A eşleşir, A B'nin quest'ini alır, A canlandır ekranında 4 adımı görür. Bu, planın "bitti tanımı"dır.
- **Yarış:** aynı quest'e iki kullanıcı aynı anda "Takas et" → biri başarı, diğeri §5.5'teki 409 mesajı.
- **Formlar:** her validasyon kuralı için alan altı mesaj; JS kapalıyken bırakma formu (dosya yüklemeyle) çalışır.
- **Erişilebilirlik:** her sayfada otomatik axe taraması; klavyeyle çekirdek döngü.
- **Görsel:** 360 px, 768 px, 1280 px; açık/koyu tema ekran görüntüsü karşılaştırması.
- **Ses:** Chrome, Firefox, Safari (iOS dahil) üzerinde kayıt → yükleme → oynatma.

---

## 13. Açık sorular

1. Bırakma login'e bağlansın mı? (Bu doküman evet varsayıyor, §5.2.)
2. ~~Ses formatı~~: karar verildi ve yapıldı (A, §6).
3. Harici ses linkleri tamamen kapatılsın mı?
4. v1 dili: sadece İngilizce mi, İngilizce + Türkçe mi? (Öneri: ikisi, sözlük dosyasıyla.)
5. Takas edilen quest'in yeni sahibine bırakanla iletişim kanalı olacak mı? Bugün yok (DM kapsam dışı). Bırakanın handle'ı görünüyor ama ulaşma yolu yok.
