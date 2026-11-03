# Build stage
FROM rust:1-bookworm AS builder
WORKDIR /app

# Copy manifests and source
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations

# Build release binary (cache layers by copying dummy main if needed)
RUN cargo build --release --locked

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# The binary carries its migrations (sqlx::migrate!) and creates the database folder itself
COPY --from=builder /app/target/release/dev-dock-manager-api /app/

# Only what differs from the defaults in src/config.rs: the database goes in the volume below, and Redis
# is the compose service called redis. Everything else uses the program's defaults (docker-compose.yml
# sets them explicitly anyway).
ENV DATABASE_URL=sqlite:///app/data/db.sqlite3
ENV REDIS_URL=redis://redis:6379

EXPOSE 8000

# Persist sqlite DB in /app/data
VOLUME ["/app/data"]

CMD ["/app/dev-dock-manager-api"]
