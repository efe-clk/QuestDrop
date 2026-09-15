FROM rust:1.82-slim AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY migrations ./migrations
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/* \
  && cargo build --locked --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/* \
  && useradd -r -u 10001 appuser
COPY --from=build /app/target/release/questdrop ./questdrop
COPY templates ./templates
RUN mkdir -p ./data/voice && chown -R appuser:appuser /app
USER appuser
ENV PORT=3000 VOICE_DIR=./data/voice
EXPOSE 3000
CMD ["./questdrop"]
