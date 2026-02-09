# Build stage
FROM rust:1-bookworm AS builder
WORKDIR /app

# Copy manifests and source
COPY Cargo.toml ./
COPY src ./src
COPY migrations ./migrations

# Build release binary (cache layers by copying dummy main if needed)
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary, migrations and entrypoint
COPY --from=builder /app/target/release/dev-dock-manager-api /app/
COPY --from=builder /app/migrations /app/migrations
COPY docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh

# Default env (override in docker-compose)
ENV BIND_ADDR=0.0.0.0:8000
ENV DATABASE_URL=sqlite:///app/data/db.sqlite3
ENV REDIS_URL=redis://redis:6379
ENV DOCKER_NETWORK=d-gui-network
ENV DOCKER_IMAGE_NAME=gui-vnc
ENV HOST_FOR_PORT_CHECK=host.docker.internal

EXPOSE 8000

# Persist sqlite DB in /app/data
VOLUME ["/app/data"]

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["/app/dev-dock-manager-api"]
