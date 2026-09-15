FROM rust:1.90-slim@sha256:618466f4caae45cd6b7b6adfa98764ad462aacf67e7149c6d277c625da9f1282 AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY migrations ./migrations
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/* \
  && cargo build --locked --release

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/* \
  && useradd -r -u 10001 appuser
COPY --from=build /app/target/release/questdrop ./questdrop
# NOTE: templates/ is compiled into the binary by Askama at build time.
RUN mkdir -p ./data/voice && chown -R appuser:appuser /app
USER appuser
ENV PORT=3000 VOICE_DIR=./data/voice
EXPOSE 3000
CMD ["./questdrop"]
