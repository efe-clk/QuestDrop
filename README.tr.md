> 🇬🇧 Click for English: [README.md](./README.md)

# QuestDrop

Yarım kalmış yan projeleri takas edilebilir quest'lere çevir. Yoruldun mu? Drop et. Başkasının yarım quest'ini al, yenilik dopaminiyle devam et.

Başarı kriteri: tanımadığın insanların açık havuzda takas yapması.

## Kapsam (MVP = A)

1. Yalın web borsası. Önce A yapılır.
2. B (Discord/Telegram botu) ve C (AI eşleşme) sonradan takılır, core değişmez.
3. Her türlü proje: kod, yazı, tasarım. Tip alanı yok, link + ses var.

## Mimari

Tek repo, API-first modüler monolit. Mikroservis yok.

```text
[Web UI] -> [API Core] -> [DB]
[Bot]   -> [API Core] (ileride, aynı API)
```

- Core dış dünyayı bilmez. Web de bot da aynı REST'i kullanır.
- Eşleşme tek interface: `Matcher.match(user, pool) -> ranked_list`. Şimdi kural-tabanlı, sonra `EmbeddingMatcher` aynı interface ile takılır.
- Event: `swap.created` için basit DB outbox tablosu. Bot bildirimi ve AI yeniden-sıralama bunu dinler.
- Auth: magic link. Dosya: link (S3 yok). Ses: 30sn mp3.

## Altyapı (kilitli)

- Web + API: Next.js App Router (TypeScript), REST
- DB: Postgres (Neon) + Prisma, pgvector kolonu hazır (şimdilik NULL)
- Auth: Auth.js email magic link
- Ses depolama: Vercel Blob
- Deploy: Vercel + Neon

Detay: [docs/stack.tr.md](./docs/stack.tr.md) | [English](./docs/stack.md)

## Dökümanlar

- Tasarım (TR): [docs/design.tr.md](./docs/design.tr.md)
- Design (EN): [docs/design.md](./docs/design.md)

## Hızlı Başlangıç

```bash
npm install
npm run dev
```

> İskelet bir sonraki commit'te. Akış için tasarım dokümanına bak.
