FROM rust:1.88-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.toml
COPY crates/dmxforge/Cargo.toml crates/dmxforge/Cargo.toml
COPY crates/dmxforge/src crates/dmxforge/src
COPY crates/dmxforge/templates crates/dmxforge/templates
COPY crates/dmxforge/migrations crates/dmxforge/migrations
COPY crates/dmxforge/static crates/dmxforge/static

RUN cargo build --release -p dmxforge

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin appuser

COPY --from=builder /app/target/release/dmxforge /app/dmxforge
COPY --from=builder /app/crates/dmxforge/static /app/static

ENV DMXFORGE_STATIC_DIR=/app/static

RUN mkdir -p /app/data && chown -R appuser:appuser /app
USER appuser

EXPOSE 3000

CMD ["/app/dmxforge"]
