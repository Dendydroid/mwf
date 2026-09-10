# autocalls.ai integration: transcript → local ONNX NLU → instruction registry → DB-backed response

## Context

The project (`ai_support_bot`) is meant to serve as the backend `autocalls.ai` calls during live calls to fetch information for the caller. Today the repo is a bare Axum + sqlx (Postgres) + Redis skeleton (`/version`, `/test-session` only, one placeholder migration, no external HTTP client, no ML code).

Researched directly against `docs.autocalls.ai`: autocalls.ai's own LLM can already do intent detection via "Custom Mid-Call Tools" (per-tool name/description/typed-parameter schema, their AI decides when to call each one). The user explicitly chose **not** to rely on that and instead wants to own the reasoning step: register a single generic tool with autocalls.ai that hands us the caller's raw request text, and run our **own** open-source HuggingFace model (via ONNX, zero-shot embedding similarity) inside this Rust service to decide which of our own registered "instructions" applies, then execute it against Postgres and return an answer. This keeps the NLU logic and the list of valid instructions entirely within this project, extensible by just adding a new instruction (name + description + handler) with no retraining needed. The user is starting from zero on the autocalls.ai side (no account/assistant/tool yet), so the account-side setup is part of this plan too, clearly separated from the code.

A pre-existing, unrelated bug was found that blocks any container rebuild and must be fixed first: `docker-compose.yml`'s `app.build.context` points at a nonexistent `mwf` directory; the real `Dockerfile` and all its `COPY` paths are repo-root-relative.

The exact JSON shape autocalls.ai expects back from a mid-call tool's endpoint is **not publicly documented** — this plan implements a reasonable first guess (`{"result": "<string>"}`) and flags it as something to confirm/adjust once the tool is reachable and tested via autocalls.ai's dashboard "Test Chat" tool.

---

## Step 0 — Fix the Docker build (prerequisite)

`docker-compose.yml`: change the `app` service's `build.context` from `mwf` to `.`. Nothing else changes here; without this, `update.sh` fails outright.

## Step 1 — New Cargo dependencies

Add to root `Cargo.toml` `[workspace.dependencies]` (referenced via `{ workspace = true }` in `app/Cargo.toml`, matching existing convention):
- `ort = { version = "=2.0.0-rc.13", default-features = false, features = ["ndarray", "load-dynamic"] }` — pin exact; `ort` 2.x has no stable release yet, so re-check crates.io at implementation time for a newer RC/stable and adjust. Use `load-dynamic` (not the default `download-binaries`) deliberately: `download-binaries` resolves a path baked at `cargo build` time relative to `OUT_DIR`, which does not survive the multi-stage Docker copy into `debian:bookworm-slim`, and would diverge between Windows dev and the Linux container anyway. `load-dynamic` instead `dlopen`s a path we supply explicitly at startup via `ort::init_from(path)?.commit()?`, which we point at a Linux `.so` in Docker and a Windows `.dll` locally via config (Step 3).
- `ndarray = "0.16"` (tensor construction for `ort`)
- `tokenizers = { version = "0.20", default-features = false }` — MiniLM's WordPiece tokenizer is fully described by its `tokenizer.json`, so the `onig` feature (a C-lib binding, a known Windows build-friction point) shouldn't be needed; verify this builds clean on the Windows dev machine and re-enable the minimal feature set if not.
- `reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }` — rustls avoids a second TLS stack alongside sqlx's `native-tls`
- `async-trait = "0.1"` — needed to make the `Instruction` trait (Step 5) object-safe for `Vec<Box<dyn Instruction>>`

`app/Cargo.toml` `[dev-dependencies]`: add `tower = { version = "0.4", features = ["util"] }` for `ServiceExt::oneshot` in route tests (tower is already a transitive dep of axum).

**Open risk, confirm during implementation, not before**: exact `ort` 2.x RC API surface (`Session::builder()` chain, `Value::from_array` tensor construction, whether `Session::run` needs `&self` or `&mut self` — this decides whether `Nlu` needs an internal `tokio::sync::Mutex`), and the ONNX Runtime binary version that RC actually links against (a mismatch fails at dylib-load time, not compile time). Check `cargo doc -p ort --open` for the pinned version before writing `nlu.rs`.

## Step 2 — ONNX Runtime binary + model file acquisition

New tool scripts (check `.claude/tools/README.md` first — it is currently empty, so these are the first entries; document both there per CLAUDE.md):

- `.claude/tools/fetch_onnxruntime.sh` — detects OS (`uname -s`), downloads the matching `onnxruntime-<platform>-x64-<version>` release asset from the official `microsoft/onnxruntime` GitHub releases (version must match whatever `ort` RC is pinned — verify), extracts just the shared library into `app/models/onnxruntime/` as a fixed name (`libonnxruntime.so` on Linux / `onnxruntime.dll` on Windows) so config paths don't need per-version updates. Idempotent (skip if already present). Windows zip extraction: shell out to `powershell.exe -NoProfile -Command "Expand-Archive -Force <zip> <dest>"` since Git Bash doesn't reliably ship `unzip` — hand-verify this on the actual dev machine.
- `.claude/tools/fetch_nlu_model.sh` — downloads `tokenizer.json` and `onnx/model_quantized.onnx` from `https://huggingface.co/Xenova/all-MiniLM-L6-v2` (an HF repo that already ships ONNX weights, avoiding any Python export step) into `app/models/all-MiniLM-L6-v2/`. Use the int8-quantized variant for a smaller/faster model — this is a threshold-gated similarity match, not a precision-critical task, so the small accuracy loss is an acceptable, reversible tradeoff.

`.gitignore`: add `/app/models/` — large, license-bearing, reproducibly fetched; don't commit.

`Dockerfile` builder stage: add `curl` to the `apt-get install` line, `COPY .claude/tools/fetch_onnxruntime.sh .claude/tools/fetch_nlu_model.sh .claude/tools/`, run both scripts early (before `COPY app ./app`, so this cacheable layer doesn't get invalidated by source changes). Runtime stage: `COPY --from=builder /workspace/app/models /app/models` alongside the existing `config`/`migrations`/`.env` copies.

**Tradeoff to flag to the user, not silently decide**: this makes the Docker build require network egress to GitHub + Hugging Face, not just crates.io. If that's unacceptable for the deployment target, an alternative is baking `app/models/` into a private volume/artifact instead — out of scope here, note it as a follow-up if it becomes a problem.

## Step 3 — Config additions

`app/config/settings.toml`, new section:
```toml
[nlu_settings]
model_path = "models/all-MiniLM-L6-v2/model_quantized.onnx"
tokenizer_path = "models/all-MiniLM-L6-v2/tokenizer.json"
similarity_threshold = 0.55
```
(Relative paths work in both `cargo run`/`cargo test` locally and the Docker runtime stage, since both have their working directory at the `app` level — confirmed by how `settings.rs` already loads `config/settings.toml` relatively today.) `similarity_threshold` is a starting guess, tune empirically against Step 9's tests.

`app/src/settings.rs`: add `NluSettings { model_path, tokenizer_path, similarity_threshold }` struct following the exact `DatabaseSettings`/`CacheSettings` pattern, plus two new **env-sourced** top-level `AppSettings` fields (like `database_url`/`cache_url`, since these are genuinely environment-specific): `onnx_dylib_path` and `autocalls_shared_secret`, each with a getter matching the existing `database_url()` style.

`.env.dist` / `.env.test.dist`: add
```
AUTOCALLS_SHARED_SECRET=changeme-generate-a-real-random-secret
ONNXRUNTIME_DYLIB_PATH=./models/onnxruntime/libonnxruntime.so
```
with a comment noting local Windows dev should override `ONNXRUNTIME_DYLIB_PATH` to `./models/onnxruntime/onnxruntime.dll` in the real (gitignored) `.env`/`.env.test`. Also document (comment only, not wired into code yet — YAGNI) an `AUTOCALLS_API_KEY=` placeholder for a possible future in-repo tool-registration script; nothing in this phase reads it.

## Step 4 — `AppError`

New `app/src/error.rs`. Existing handlers (`version`, `session`) keep their current `Result<Json<T>, StatusCode>` unchanged — they have no real failure diversity to justify a shared type. The new subsystem does (auth rejection vs. DB failure vs. NLU/classification failure vs. bad request all need distinct handling), which is the actual justification:
```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("nlu error: {0}")]
    Nlu(#[from] anyhow::Error),
    #[error("bad request: {0}")]
    BadRequest(String),
}
```
`impl IntoResponse for AppError`: `Unauthorized` → 401, `BadRequest` → 400, everything else → 500 with a generic body; log the real error server-side via `tracing::error!` first — this endpoint is hit by a third party, don't leak internals. Add `mod error;` to both `main.rs` and `lib.rs`.

## Step 5 — Instruction registry (Strategy pattern)

New `app/src/instructions/mod.rs`. This is exactly CLAUDE.md's "generalize via design pattern" case (heterogeneous, independently-addable behaviors dispatched at runtime without an if/match chain) — **top of file must carry the pattern-explanation comment**:
```rust
// Pattern: Strategy. Each `Instruction` is an interchangeable strategy for
// handling one caller-request intent; `InstructionRegistry` is the context
// that picks a strategy at runtime via NLU similarity (nlu.rs) instead of a
// branching chain keyed on request content. New capabilities are added by
// writing a new `Instruction` impl and registering it in
// `InstructionRegistry::new()` — dispatch code itself never changes.

#[async_trait::async_trait]
pub trait Instruction: Send + Sync {
    fn name(&self) -> &'static str;
    /// Embedded once at startup, compared against caller utterances — phrase
    /// naturally, matching quality depends on this, not keyword stuffing.
    fn description(&self) -> &'static str;
    async fn execute(&self, state: &AppState, raw_text: &str) -> Result<InstructionOutcome, AppError>;
}

pub struct InstructionOutcome { pub message: String }

pub struct InstructionRegistry { instructions: Vec<Box<dyn Instruction>> }
impl InstructionRegistry {
    pub fn new() -> Self {
        Self { instructions: vec![
            Box::new(system_status::SystemStatusInstruction),
            Box::new(business_hours::BusinessHoursInstruction),
            Box::new(business_location::BusinessLocationInstruction),
        ]}
    }
    pub fn all(&self) -> &[Box<dyn Instruction>] { &self.instructions }
    pub fn get(&self, name: &str) -> Option<&dyn Instruction> {
        self.instructions.iter().find(|i| i.name() == name).map(|b| b.as_ref())
    }
}
```
One file per instruction: `app/src/instructions/{system_status,business_hours,business_location}.rs` (bodies in Step 8). Add `mod instructions;` to `main.rs` and `lib.rs`.

`AppState` (`app/src/app.rs`) gains an `instructions: InstructionRegistry` field, constructed first in `AppState::new()` (cheap, no I/O) since `Nlu::load` needs it.

## Step 6 — NLU / embedding-similarity module

New `app/src/nlu.rs` (plain service module like `cache.rs`/`db.rs`, no pattern-comment needed):
```rust
pub struct Nlu {
    tokenizer: tokenizers::Tokenizer,
    session: ort::session::Session,
    instruction_embeddings: Vec<(String, Vec<f32>)>, // L2-normalized, name -> embedding
    similarity_threshold: f32,
}
impl Nlu {
    pub async fn load(settings: &AppSettings, registry: &InstructionRegistry) -> anyhow::Result<Self> { .. }
    pub async fn classify(&self, text: &str) -> Option<(String, f32)> { .. }
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        // tokenize -> run ONNX session -> mean-pool last_hidden_state using the
        // attention mask (sentence-transformers pooling for MiniLM — NOT [CLS])
        // -> L2-normalize so dot product == cosine similarity
    }
}
```
- `ort::init_from(settings.onnx_dylib_path())?.commit()?` must run exactly once per process before any other `ort` call — guard with `std::sync::OnceLock` inside `nlu.rs` so both `AppState::new()` and Step 9's standalone test construction don't double-init.
- `classify()` must run the actual tokenize+inference inside `tokio::task::spawn_blocking` — ONNX CPU inference is synchronous, blocking a Tokio worker thread inline is bad practice under real concurrency even though a single MiniLM forward pass is low-ms.
- Mean-pooling with the attention mask is required for correct embeddings from this model family; get it wrong and similarity scores degrade silently rather than erroring — Step 9's known-phrase tests are the actual safety net here, not code review.

`AppState` gains `nlu: Nlu`, built after `instructions`: `let nlu = Nlu::load(&settings, &instructions).await.expect("Failed to load NLU model");` — panic-on-failure matches the existing `Database::new` startup pattern.

## Step 7 — Route, DTOs, and correctly-scoped middleware

**Verified by reading `main.rs`/`api.rs`/`middleware.rs` directly**: `session_middleware` today is applied *globally* in `main.rs` via `.layer(middleware::from_fn_with_state(...))` after the router is built, so it currently runs on every route including `/version`. For the new webhook route this is wrong — it's a server-to-server call from autocalls.ai with no cookies, and running the cookie-session middleware on it would create and persist a throwaway Redis session on every single call. Fix by restructuring `router()` to scope each middleware to the routes that need it via `.route_layer` (which axum 0.7 preserves correctly through `.merge()`), rather than applying `session_middleware` globally in `main.rs`:

`app/src/routes/api.rs`:
```rust
pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let browser_routes = Router::<Arc<AppState>>::new()
        .route("/version", get(version))
        .route("/test-session", get(session))
        .route_layer(middleware::from_fn_with_state(state.clone(), session_middleware));

    let autocalls_routes = Router::<Arc<AppState>>::new()
        .route("/autocalls/handle-request", post(handle_autocalls_request))
        .route_layer(middleware::from_fn_with_state(state, autocalls_auth_middleware));

    Router::<Arc<AppState>>::new().merge(browser_routes).merge(autocalls_routes)
}
```
`app/src/main.rs`: drop the global `session_middleware` `.layer(...)` call, change to `.merge(routes::api::router(state.clone()))`, keep `TraceLayer`/`CompressionLayer`/`CorsLayer` global (those are fine for all routes).

New auth middleware in `app/src/routes/middleware.rs` (alongside `session_middleware`, unchanged):
```rust
const AUTOCALLS_SECRET_HEADER: &str = "X-Autocalls-Secret";

pub async fn autocalls_auth_middleware(
    State(state): State<Arc<AppState>>, req: Request, next: Next,
) -> Result<Response, AppError> {
    let provided = req.headers().get(AUTOCALLS_SECRET_HEADER).and_then(|v| v.to_str().ok());
    let expected = state.settings.autocalls_shared_secret();
    if matches!(provided, Some(p) if constant_time_eq(p.as_bytes(), expected.as_bytes())) {
        Ok(next.run(req).await)
    } else {
        Err(AppError::Unauthorized)
    }
}
// Manual constant-time compare to avoid a timing side-channel; not worth pulling in `subtle` for one comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool { .. }
```

DTOs + handler in `app/src/routes/api.rs` (same local-struct convention as `VersionResponse`/`TestSessionResponse`):
```rust
#[derive(Deserialize)]
struct AutocallsHandleRequestPayload { request_text: String }

#[derive(Serialize)]
struct AutocallsHandleRequestResponse {
    // Best-guess contract — autocalls.ai's expected shape isn't publicly
    // documented. Verify via their dashboard Test Chat tool (Step 10.6) and
    // adjust this struct once confirmed.
    result: String,
}

async fn handle_autocalls_request(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AutocallsHandleRequestPayload>,
) -> Result<Json<AutocallsHandleRequestResponse>, AppError> {
    let message = match state.nlu.classify(&payload.request_text).await {
        Some((name, _score)) => match state.instructions.get(&name) {
            Some(instruction) => instruction.execute(&state, &payload.request_text).await?.message,
            None => { tracing::error!(instruction = %name, "NLU matched a name not in the registry"); fallback_message() }
        },
        None => fallback_message(),
    };
    Ok(Json(AutocallsHandleRequestResponse { result: message }))
}
fn fallback_message() -> String {
    "I'm sorry, I don't have information about that. Let me connect you with a team member.".into()
}
```

## Step 8 — First 3 starter instructions (proof of DB pipeline)

Uncomment `sqlx::migrate!("../migrations").run(&state.db.pool()).await?;` in `main.rs` (currently disabled) — this feature is the first thing that actually needs real schema present at runtime.

New migration `migrations/<timestamp>_create_business_info.up.sql` / `.down.sql`:
```sql
CREATE TABLE IF NOT EXISTS business_info (
    key VARCHAR(50) PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO business_info (key, value) VALUES
    ('hours', 'We are open Monday to Friday, 9 AM to 6 PM.'),
    ('address', '123 Example Street, Springfield.')
ON CONFLICT (key) DO NOTHING;
```
Queries use `sqlx::query_as::<_, T>(...)` (runtime-checked) rather than the `query!` compile-time macro — the repo has no `DATABASE_URL`-at-compile-time / `.sqlx` offline cache set up, and adding that is out of scope here.

1. `app/src/instructions/system_status.rs` — `SystemStatusInstruction` (`name = "system_status"`), reuses the existing placeholder `test_table`: `SELECT COUNT(*)::BIGINT FROM test_table`. Description: "Check whether the phone support system is online and can reach its database — caller asks if anyone is there or if the system is working." Pure end-to-end sanity check.
2. `app/src/instructions/business_hours.rs` — `BusinessHoursInstruction` (`name = "business_hours"`), `SELECT value FROM business_info WHERE key = 'hours'`. Description: "Tell the caller the business's opening hours or when it is open or closed."
3. `app/src/instructions/business_location.rs` — `BusinessLocationInstruction` (`name = "business_location"`), same table, `key = 'address'`. Description: "Tell the caller the business's address or physical location."

## Step 9 — Tests

Follows existing convention exactly (`app/src/lib.rs`, manual `Runtime::block_on`, real Postgres/Redis via `.env.test`, no new test framework). **Verified**: `lib.rs` currently declares only `mod cache/db/factory/session/settings` — it does *not* currently include `app`/`routes`. Add `mod app; mod routes; mod error; mod instructions; mod nlu;` to `lib.rs` too (duplicated compilation between the `app` binary and this lib crate — already how `db`/`cache`/`session`/`settings` work today, not a new pattern).

1. **NLU correctness** — proves the embedding pipeline actually classifies, not just compiles:
```rust
#[test]
fn nlu_classifies_known_phrases_to_expected_instructions() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let settings = AppSettings::load();
        let registry = InstructionRegistry::new();
        let nlu = Nlu::load(&settings, &registry).await.expect("nlu load failed");
        for (phrase, expected) in [
            ("what time do you open", "business_hours"),
            ("where are you located", "business_location"),
            ("is anyone there, are you online", "system_status"),
        ] {
            assert_eq!(nlu.classify(phrase).await.map(|(n, _)| n), Some(expected.to_string()), "phrase: {phrase}");
        }
        assert!(nlu.classify("what's the weather like on mars today").await.is_none());
    });
}
```
Document in `.claude/CLAUDENOTES.md` that this requires `app/models/` populated first (run both fetch scripts once locally) — it fails loudly, not silently, if missing.

2. **Route + auth middleware**, using `tower::util::ServiceExt::oneshot` against the real `Router` built the same way `main.rs` builds it (new precedent, smallest addition consistent with not inventing a framework):
```rust
#[test]
fn autocalls_route_requires_correct_shared_secret() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let state = Arc::new(AppState::new().await);
        let app = routes::api::router(state.clone()).with_state(state.clone());
        let body = r#"{"request_text":"what time do you open"}"#;

        assert_eq!(app.clone().oneshot(post_req(None, body)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(post_req(Some("wrong-secret"), body)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(post_req(Some(state.settings.autocalls_shared_secret()), body)).await.unwrap().status(), StatusCode::OK);
        // scoping check: session_middleware must NOT run on this route, and auth must NOT leak onto /version
        assert_eq!(app.oneshot(Request::builder().uri("/version").body(Body::empty()).unwrap()).await.unwrap().status(), StatusCode::OK);
    });
}
```

3. Run `cargo fmt` and the full test suite (`cargo test`) before considering this done, per CLAUDE.md.

## Step 10 — autocalls.ai account/dashboard checklist (outside this repo, starting from zero)

1. Create an autocalls.ai account, get the account API key (`Authorization: Bearer <key>`).
2. Create an Assistant (dashboard or `POST` create-assistant API). System prompt should explicitly tell it to defer to the tool rather than answer itself, e.g.: *"For any caller request needing business information, a system-status check, or an action, call the `handle_request` tool with the caller's exact wording as `request_text`, then relay its response. Do not answer such requests from your own knowledge."*
3. Create the one generic mid-call tool via `POST /api-reference/mid-call-tools/create-tool`:
   - `name`: `handle_request`
   - `description`: "Handles any caller request or question by looking it up in the business's backend system. Call this whenever the caller asks something requiring business-specific information or an action."
   - `endpoint`: `https://<our-public-domain>/autocalls/handle-request` — **note: this needs a public HTTPS URL, and this repo currently has no TLS/reverse-proxy set up** (only plain HTTP on `APP_PORT`). Resolve via a tunnel (e.g. ngrok) for initial testing, or add TLS termination (Caddy/nginx + Let's Encrypt) before going live — out of scope for this plan, but blocks Step 10.6/10.7 until resolved.
   - `method`: `POST`, `body_format`: `json`, `timeout`: ~8s (comfortable margin over expected low-ms inference + DB latency, under their 30s cap)
   - `headers`: `[{ "name": "X-Autocalls-Secret", "value": "<same value as AUTOCALLS_SHARED_SECRET in .env>" }]`
   - `schema`: `[{ "name": "request_text", "type": "string", "description": "The caller's exact request or question, verbatim", "required": true }]`
4. Attach the tool to the assistant via its `tool_ids` array.
5. Provision a phone number and point it at the assistant (follow autocalls.ai's own provisioning flow).
6. Use the dashboard's "Test Chat" tool to trigger `handle_request` and inspect exactly what our endpoint receives and what response shape is expected — **this resolves the open `AutocallsHandleRequestResponse` question from Step 7**; adjust the struct once confirmed.
7. Test with a real inbound call once Test Chat confirms the contract.

---

## Open risks (explicitly flagged, not silently resolved)

- `ort` has no stable 2.0 release yet (`2.0.0-rc.13` newest) — pin exact, re-verify at implementation time; exact Session/tensor-building API and ONNX Runtime version pairing must be confirmed against `cargo doc` for the pinned version, not assumed from this plan.
- `tokenizers` without the `onig` feature, and the PowerShell-based zip extraction in `fetch_onnxruntime.sh`, are expected-to-work but not hands-on verified.
- autocalls.ai's mid-call-tool response JSON contract is genuinely undocumented — `{"result": "<string>"}` is a placeholder pending Step 10.6.
- No TLS/reverse-proxy exists yet, but the tool endpoint must be public HTTPS — blocks live end-to-end testing until resolved (separately from this plan).
- `similarity_threshold = 0.55` needs empirical tuning against real phrasing.

## Critical files

- `app/src/nlu.rs` (new) — highest-risk piece, ONNX/embedding classification core
- `app/src/instructions/mod.rs` (new) — Strategy-pattern registry, needs the CLAUDE.md pattern comment
- `app/src/routes/api.rs` — new route/DTOs, `router()` now takes `Arc<AppState>`
- `app/src/routes/middleware.rs` — new `autocalls_auth_middleware`
- `app/src/app.rs` — `AppState` gains `instructions`/`nlu`
- `app/src/main.rs` — router restructuring (scoped middleware, not global), migrations re-enabled
- `Dockerfile`, `docker-compose.yml` — Step 0 fix + model/runtime packaging
- `app/src/settings.rs`, `app/config/settings.toml` — new config surface
- `.claude/tools/fetch_onnxruntime.sh`, `.claude/tools/fetch_nlu_model.sh` (new, documented in `.claude/tools/README.md`)

## Verification

1. Run `.claude/tools/fetch_onnxruntime.sh && .claude/tools/fetch_nlu_model.sh` locally once.
2. `cargo fmt && cargo test` — all existing tests plus the two new ones (Step 9) pass.
3. `./update.sh` — container rebuilds successfully (proves Step 0 fix + Dockerfile model-fetch stage work).
4. `curl -X POST localhost:$APP_PORT/autocalls/handle-request -H "X-Autocalls-Secret: <secret>" -H "Content-Type: application/json" -d '{"request_text":"what time do you open"}'` returns a 200 with the business hours; a request with a missing/wrong secret returns 401; `/version` still works with no secret header (proves middleware scoping didn't regress existing routes).
5. Complete the Step 10 account checklist and use autocalls.ai's Test Chat to confirm/adjust the response contract, then place a real test call.