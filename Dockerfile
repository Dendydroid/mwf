# -bookworm must match the debian:bookworm-slim runtime below: the default
# rust:1.95-slim is trixie (glibc 2.41) and produces a binary bookworm
# (glibc 2.36) cannot load.
FROM rust:1.95-slim-bookworm AS builder

WORKDIR /workspace

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies separately from source
COPY Cargo.toml Cargo.lock ./
COPY app/Cargo.toml app/
RUN mkdir -p app/src && echo "fn main() {}" > app/src/main.rs
RUN cargo build --release --bin app
RUN rm -rf app/src

COPY app ./app
COPY migrations ./migrations
# The mimic client page is `include_str!`d into the binary (routes/api.rs), so
# it has to exist at build time - without this the release build fails here
# rather than at runtime.
COPY client ./client
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
# The default prompt, so the image runs on its own. docker-compose.yml mounts
# the host copy over it, which is what makes editing a prompt a save rather than
# a rebuild.
COPY --from=builder /workspace/app/prompts /app/prompts
COPY --from=builder /workspace/migrations /app/migrations
# .env is copied from the build context, not the builder stage, so that
# editing it does not invalidate the cached dependency build above.
COPY .env /app/.env

EXPOSE 8080

CMD ["./server"]
