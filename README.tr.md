> 🇬🇧 Click for English: [README.md](./README.md)

# QuestDrop (Rust)

Yarım kalmış yan projeleri takas edilebilir quest'lere çevir. Yoruldun mu? Drop et. Başkasının yarım quest'ini al, yenilik dopaminiyle devam et.

Başarı kriteri: tanımadığın insanların açık havuzda takas yapması.

## Kapsam (MVP = A)

1. Yalın web borsası. Önce A yapılır.
2. B (Discord/Telegram botu) ve C (AI eşleşme) sonradan takılır, core değişmez.
3. Her türlü proje: kod, yazı, tasarım. Tip alanı yok, link + ses var.

## Mimari

Tek repo, Rust ile API-first modüler monolit. Mikroservis yok.

```text
[Web UI - Askama+HTMX] -> [API Core - Axum] -> [Postgres+sqlx]
[Bot] -----------------> [API Core - Axum] (ileride, aynı API)
```

- Core dış dünyayı bilmez. Web de bot da aynı REST'i kullanır (`/v1/`).
- Eşleşme tek trait: `trait Matcher { fn match(...) -> ranked_list }`. Şimdi kural-tabanlı, sonra `EmbeddingMatcher` impl.
- Event: `swap.created` için basit DB outbox tablosu. Bot bildirimi ve AI yeniden-sıralama bunu dinler.
- Auth: email magic link. Dosya: link (S3 yok). Ses: `./data/voice` içinde 30sn mp3.

## Altyapı (kilitli - Rust)

- Web + API: Axum + Askama + HTMX, tek binary
- DB: Postgres + sqlx, pgvector rezerve
- Auth: magic link (lettre) + cookie session
- Ses: local static dosya
- Deploy: Docker + Fly.io

Detay: [docs/stack.tr.md](./docs/stack.tr.md) | [English](./docs/stack.md)

## Dökümanlar

- Tasarım (TR): [docs/design.tr.md](./docs/design.tr.md)
- Plan (TR): [docs/plan.tr.md](./docs/plan.tr.md)
- Design (EN): [docs/design.md](./docs/design.md)
- Plan (EN): [docs/plan.md](./docs/plan.md)

## Hızlı Başlangıç

```bash
cargo run
```

> İskelet bir sonraki commit'te. Fazlar için plana bak.
