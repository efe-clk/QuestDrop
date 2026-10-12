FROM rust:1.90-slim@sha256:618466f4caae45cd6b7b6adfa98764ad462aacf67e7149c6d277c625da9f1282 AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY migrations ./migrations
COPY i18n ./i18n
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/* \
  && cargo build --locked --release

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/* \
  && useradd -r -u 10001 appuser
COPY --from=build /app/target/release/questdrop ./questdrop
# Outbox delivery worker (docker-compose.yaml runs it as the `bot` service).
COPY --from=build /app/target/release/bot ./bot
COPY static ./static
# NOTE: templates/ and i18n/ are compiled into the binary at build time;
# static/ (CSS, JS, vendored HTMX) is served from disk.
RUN mkdir -p ./data/voice && chown -R appuser:appuser /app
USER appuser
ENV PORT=3000 VOICE_DIR=./data/voice
EXPOSE 3000
# /ready is 200 only when Postgres answers (the server keeps running without
# a DB, so /health alone would hide a missing database).
HEALTHCHECK --interval=15s --timeout=5s --start-period=20s --retries=5 \
  CMD curl -fsS "http://127.0.0.1:${PORT}/ready" || exit 1
CMD ["./questdrop"]
