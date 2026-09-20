# Handoff — ai_assistant (vLLM intent endpoint + mimic client)

**Session date:** 2026-09-17 · **Repo:** `H:\rust\ai_assistant` (branch `master`)
**Reference project:** `H:\rust\ai_autocalls_rust` — the earlier, larger implementation this one borrows ideas from.

---

## Context

`ai_assistant` had a working axum + Postgres + Redis skeleton and an `ai_prompt_loop`
function in `main.rs` that read stdin, sent each line to vLLM under a fixed system
prompt, and printed the raw response. It was awaited unconditionally before
`axum::serve`, so the HTTP server never actually started.

This session turned that loop into a real HTTP API, added a browser client that
speaks to it, fixed a 502 that showed up under load, added file logging on a
mounted volume, and moved the system prompt out of the binary so prompts can be
iterated on without a rebuild. Everything below is **implemented and verified
running** unless marked otherwise.

The last part of the session was research only (no code): how to extend this into
multi-step form filling — booking, authentication, per-call memory. That design is
summarised at the end; the user is writing their own concept doc before anything
is built.

---

## What exists now

### Endpoints

| route | purpose |
|---|---|
| `POST /assistant/handle-request` | `{request_text, language?}` → the model's decision, JSON |
| `GET /mimic-client` | the browser client, `include_str!`d from `client/index.html` |
| `GET /version`, `GET /test-session` | pre-existing, untouched |

Response shape — `answer` plus **whatever else the prompt's schema asked for**,
flattened, in schema order:

```json
{"answer":"I can help with that…","selected_function":"check_invoices","confidence":0.95,
 "language_detected":"en","anger_level":0.7,"is_hanging_up":false,"is_greeting":false,
 "requires_clarification_question":true}
```

### Files changed / added

| path | what |
|---|---|
| `app/src/vllm.rs` | rewritten: `SystemPrompt` (file-backed, reloads per turn), `VllmClient`, `ActionDecision`, `VllmError`, 14 tests |
| `app/src/routes/api.rs` | the two new routes, `AssistantResponse`, page↔route contract test |
| `app/src/error.rs` | **new** — `ApiError` → `{"error": "..."}`; 400 / 502 / 504 |
| `app/src/app.rs` | `AppState::new(settings)` now *takes* settings; holds `llm: VllmClient` |
| `app/src/main.rs` | `init_tracing` (stdout + optional rolling file), settings load first, `--prompt-loop` flag |
| `app/src/settings.rs` | `llm_url`, `llm_model`, `llm_api_key`, `llm_system_prompt_file`, `log_dir`, `LlmSettings` |
| `app/config/settings.toml` | defaults for the above; `timeout_seconds=120`, `temperature=0.0`, `max_tokens=256` |
| `app/prompts/system_prompt.txt` | **new** — the system prompt, formerly a Rust `const` |
| `app/prompts/README.md` | **new** — rules for editing prompts |
| `client/index.html` | **new** — mimic client (speech in/out, generic badges) |
| `Cargo.toml` | `tracing-appender`; `serde_json` with `preserve_order` |
| `Dockerfile` | `COPY client`, `COPY prompts` (both needed at build time) |
| `docker-compose.yml` | `LOG_DIR`, `LLM_SYSTEM_PROMPT_FILE`, volumes for `./logs` and `./app/prompts` |
| `.env`, `.env.dist`, `.env.test`, `.env.test.dist` | `RUST_LOG` consolidated, `LLM_SYSTEM_PROMPT_FILE` added |
| `.gitignore`, `logs/.gitkeep` | ignore rotated logs, keep the mount point |

**Nothing is committed.** The whole session is in the working tree.

---

## Four design decisions worth not re-litigating

**1 · The decision is pass-through, not a fixed struct.**
`ActionDecision` names exactly one key — `humanlike_sentence_answer`, the sentence
the caller hears — and carries everything else in a flattened
`serde_json::Map`. Add a key to the prompt schema and it reaches the API response
and the client badges with no Rust change. `serde_json`'s `preserve_order` feature
keeps keys in schema order; dropping that feature silently alphabetises the badge
row (there is a test for it).

Removing `humanlike_sentence_answer` from the schema fails every turn with a
`missing field` error. That is deliberate.

**2 · The prompt is a file, re-read every turn.**
`app/prompts/system_prompt.txt`, pointed at by `LLM_SYSTEM_PROMPT_FILE` (relative
to the working dir, like `config/settings.toml`: `app/` locally, `/app/` in the
container). `docker-compose.yml` mounts `./app/prompts` read-only over the image
copy, so editing the prompt is a save — no restart, no rebuild. Read is
`tokio::fs`, microseconds next to a 1–3 s GPU call.

A missing / mid-save / empty file logs a warning and keeps the last good text.
An unreadable prompt **at startup** aborts the process.

Changing *which file* is read still needs a restart (the path is read once).

**3 · The 502, and why the settings are what they are.**
The original failure was not vLLM being down. `latency=30000 ms` matched
`timeout_seconds = 30` exactly — the client gave up on a slow turn. Three causes,
all fixed: no `max_tokens` (generation was unbounded to the 4096 context), a
timeout sized for the good case, and `reqwest`'s `Display` hiding the cause so a
timeout and a refused connection logged identically. Now: `max_tokens = 256`,
`timeout_seconds = 120`, a distinct `VllmError::Timeout {budget_seconds}` → **504**
(retryable) vs 502 for a genuinely broken upstream, and `VllmError::Truncated` so a
cut-off answer never reads as "the model emitted bad JSON".

Normal turns measure 0.6–2.8 s. The cap is ~2.5× the observed answer length.

**4 · Logging.**
`tracing-appender`, daily rolling, ANSI off, to `LOG_DIR` (`/var/log/app` in the
container → `./logs` on the host). Settings load *before* tracing init, because
`LOG_DIR` decides where logging goes.

That re-ordering exposed a trap: `.env` had three `RUST_LOG` lines and the last
one won — `mwf=trace,tower_http=debug`, naming a crate this project doesn't have,
which filters out the app's own logs. Harmless before (tracing started first), it
would have made the new log files empty. Now one line:
`RUST_LOG=info,sqlx=warn,tower_http=info`.

---

## How to run and verify

```bash
docker compose up -d --build app      # ~4 min; db/redis/vllm assumed already up
curl http://localhost:8080/version
```

- Mimic client: <http://localhost:8080/mimic-client> (Chrome/Edge for the mic;
  the typed input exercises the identical path without one)
- Terminal loop: `cargo run --bin app -- --prompt-loop` (from `app/`)
- Tests: `cargo test --workspace --lib --bins` → **19 lib + 15 bin** (the `vllm`
  tests compile into both targets)
- Logs: `./logs/app.log.<date>`

Running locally instead of in the container needs `DATABASE_URL`, `REDIS_URL` and
`LLM_URL` overridden to `localhost`, because `.env` points at the compose
hostnames.

### Verified live this session

- Classifications: `cancel_subscription`, `check_invoices`, `clean_order`,
  `small_talk`, `fallback_human`, `prompt_injection_attempt`
- Signals track: "WHY IS MY BILL WRONG AGAIN" → `anger_level 0.7`; "goodbye" →
  `is_hanging_up true`; "hello there" → `is_greeting true`
- Hot reload: added a `repair_request` function plus `urgency` and
  `mentions_money` to the prompt file mid-session; next turn used them; file then
  restored. Log shows `bytes=888 → 998 → 888`.
- Empty `request_text` → `400 {"error":"request_text must not be empty"}`
- Timeout classification is covered by a test that points the client at a socket
  which accepts and never answers.

---

## Known gaps / next actions

| item | note |
|---|---|
| **PII in logs** | `request_text` is logged verbatim. On real calls that is personal data (GDPR retention/erasure). Redact before this touches a phone line. |
| **No per-call state** | `/assistant/handle-request` is stateless by design. Multi-step needs a call session — see below. |
| **Nothing committed** | whole session is uncommitted working tree |
| `sqlx::migrate!` | still commented out in `main.rs`, as it was found |
| `bash.exe.stackdump` | junk file in repo root, untracked |
| Cookie session | the global `session_middleware` still runs on these routes; unused by the endpoint |

---

## Research: multi-step forms (no code written)

The user asked whether per-call session + conversation memory + slot placeholders
+ per-step completion conditions is "how it's usually done". It is — this is
  task-oriented dialogue / slot filling, with a lineage from VoiceXML (1999) through
  Rasa forms, Dialogflow CX pages/parameters, Amazon Lex slots. LLMs changed the
  understanding and phrasing layers, not the state layer.

**`ai_autocalls_rust` already implements this** and is the best reference:
`call/session.rs` (Redis call session, TTL), `forms/flow.rs` (stage machine),
`forms/parsers/` (typed slot parsers), `events/subscribers/form_*.rs`
(ask → parse → confirm → retry → hand off), `forms/delivery.rs` (persist before
telling the caller it's done).

Key points from the discussion:

- **Transcript is evidence; the slot map is state.** Keep a rolling window of 4–6
  turns for reference resolution ("make it an hour later"), but never let the
  model re-derive the booking from the transcript each turn.
- **Slots need provenance**: `verified` (written only from an API/tool result,
  e.g. `customer_id`) vs `collected` (needs parse → normalise → confirm). A
  hallucinated customer id is a data-protection incident, not a UX bug.
- **The model extracts; code decides.** Step transitions are a deterministic
  predicate (`filled && valid && (!needs_confirm || confirmed)`), never the model
  answering "are we done?".
- **Commits must be idempotent** (`call_id` + slot hash), because telephony
  webhooks retry.
- **Voice realities**: read back dates/amounts/IDs, spell IDs digit by digit,
  count no-match/no-input and hand off after N.

### Prompt layering (the user's last question)

Order layers most-static → most-volatile, which is also what keeps vLLM's prefix
cache useful (≈80% hit rate observed):

```
0 identity / tone / anti-injection     ~never changes
1 output contract (shared schema keys)  when the schema changes
2 capabilities (form + function list)   when a form is added
3 form-level policy                     per form
4 step instruction + its own slots      per step
5 runtime state (slots, missing, window) every turn — rendered, never a file
```

Suggested storage: front-matter + markdown body, one file per step, numbered
filenames for visible ordering:

```
prompts/system/00_identity.md
prompts/system/10_output_contract.md
prompts/forms/booking/form.toml        # manifest: steps, slots, validators
prompts/forms/booking/20_collect_date.md
```

The rule that prevents the worst bug: **render the schema text in the prompt from
the same manifest the validator uses** — never maintain the two by hand. Assembled
schema = contract keys ∪ current step's slots. Use explicit `{{placeholders}}` with
load-time errors on unknown names (minijinja/tera, not `str::replace`). Log a hash
of the assembled prompt per turn so a transcript can be traced to the exact prompt
version.

---

## Proposed next action

Copy this document into the repository as `HANDOFF.md` (or `docs/HANDOFF.md`) so
it travels with the code, since it currently lives outside the project in
`~/.claude/plans/`. No other changes — the multi-step work waits on the user's own
concept document.
