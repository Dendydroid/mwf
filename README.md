# mwf (mini-web-framework)

Minimal Rust web framework skeleton using **Axum**, **sqlx** (Postgres), and **Redis**.

## Stack

| Concern       | Crate / Tool                    |
|---------------|---------------------------------|
| HTTP          | axum 0.7                        |
| Database      | sqlx 0.7 + PostgreSQL 16        |
| Migrations    | sqlx migrate (embedded)         |
| Cache/Session | redis 0.25 (connection manager) |
| Config        | config + dotenvy                |
| Tracing       | tracing + tracing-subscriber    |
| Errors        | thiserror + anyhow              |

## Project layout

```
.
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
├── .env.example
├── migrations/
│   └── 20240101000000_create_users.sql
└── src/
    ├── main.rs          # server boot, AppState
    ├── config/          # AppConfig (env-based)
    ├── db/              # PgPool + migrations runner
    ├── cache/           # Redis ConnectionManager + helpers
    ├── middleware/
    │   └── error.rs     # AppError → HTTP response
    └── routes/
        ├── health.rs    # GET /health
        └── api.rs       # example: GET /api/users/:id
```

## Quick start

### With Docker Compose (recommended)

```bash
cp .env.example .env
docker compose up --build
```

The app starts on `http://localhost:8080`.

### Local (cargo run)

Start Postgres and Redis locally (or point `DATABASE_URL` / `REDIS_URL` at
the Docker containers), then:

```bash
cp .env.example .env
# edit .env if needed
cargo run
```

### Install sqlx-cli for manual migration work

```bash
cargo install sqlx-cli --no-default-features --features native-tls,postgres
sqlx migrate run          # apply
sqlx migrate revert       # roll back latest
sqlx migrate add <name>   # create new migration file
```

## Endpoints

| Method | Path            | Description                        |
|--------|-----------------|------------------------------------|
| GET    | `/health`       | DB + Redis liveness check          |
| GET    | `/api/users/:id`| Fetch user by UUID (cache-through) |

## Adding a new route

1. Create `src/routes/your_resource.rs`
2. Define `pub fn router() -> Router<AppState>`
3. Register it in `src/main.rs`: `.merge(routes::your_resource::router())`

## Configuration

All config is read from environment variables:

| Variable       | Default       | Description                  |
|----------------|---------------|------------------------------|
| `PORT`         | `8080`        | HTTP bind port               |
| `DATABASE_URL` | **required**  | Postgres connection string   |
| `REDIS_URL`    | **required**  | Redis connection string      |
| `APP_ENV`      | `development` | `development` or `production`|
