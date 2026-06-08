FROM rust:1.95-slim AS builder

WORKDIR /workspace

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies separately from source
COPY Cargo.toml Cargo.lock .env ./
COPY app/Cargo.toml app/
RUN mkdir -p app/src && echo "fn main() {}" > app/src/main.rs
RUN cargo build --release --bin app
RUN rm -rf app/src

COPY app ./app
RUN touch app/src/main.rs
RUN cargo build --release --bin app

# ── Stage 2: Runtime ──────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
COPY --from=builder /workspace/target/release/app /app/server
COPY --from=builder /workspace/app/config /app/config
COPY --from=builder /workspace/migrations /app/migrations
COPY --from=builder /workspace/.env /app/.env

EXPOSE 8080

CMD ["./server"]
