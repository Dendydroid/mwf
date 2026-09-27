# Context — ai_assistant

**Updated:** 2026-09-26 (evening) · **Repo:** `H:\rust\ai_assistant` (branch `master`, last commit `b359332 sucecess 1`,
this session's later work uncommitted) · **Stack:** Rust / axum 0.7 · Redis (call sessions) · Postgres (connected, not
used by the assistant) · vLLM serving `Qwen/Qwen2.5-7B-Instruct-AWQ` over the OpenAI-compatible API, on an RTX 3080 Ti
(12 GB) that also drives the Windows desktop
**Reference project:** `H:\rust\ai_autocalls_rust` · **Background:** `docs/call-assistant-handbook.html`

The single-prompt design of 2026-09-17 (`SystemPrompt`, `ActionDecision`, prompt hot-reload) is gone; its handoff is
`git show fd90a3d:context.md`.

---

## What it is

The backend of a phone assistant. Each caller utterance is one HTTP request, tied to a call by the `x-call-id` header.
Every turn runs two LLM "machines":

1. **Intent matcher** (machine #1) classifies the utterance into a `CallerIntent` and extracts a form value.
2. **Response formulator** (machine #2, in three variants) writes the sentence the caller hears.

Between them, deterministic Rust code changes the call's state (forms and their steps) through the event system and
tells machine #2 exactly what to say next. **The model extracts, code decides.**

Supported: current weather in Berlin, EUR→UAH rate, one form (doctor appointment) filled field by field with each
value confirmed, questions about the form, a refusal for calendar questions, repeat, end call, transfer to a human,
and a polite refusal listing what is supported. A completed form is POSTed to another project.

---

## One turn, end to end

```
POST /assistant/handle-request   header x-call-id: <id>   body {"request_text": "...", "language": "en-US"?}
│
├─ call_session_middleware — a route layer on this route only (routes/api.rs::router)
│    missing/empty x-call-id → 400 {"error": "x-call-id header is required"}
│    state.call_locks.lock(call_id)            per-call lock, held until the session is saved
│    CallSession::from_or_new(call_id)         Redis key call_id.<id>
│    dispatch CallSessionLoaded → InitialContextHandler fills initial_context (current_time)
│    after the handler: save_slots(ttl 3600 s) on every request → sliding TTL
│
└─ handle_assistant_request (routes/api.rs)
     1. Machine::intent_matcher().query::<ExtractedIntent>   unknown label → Unsupported (warn)
     2. GetInformation → selected.fetch(&http_client)         (handlers can't await)
     3. IntentExtracted → CallStateHandler: changes CallState, writes backend_context (+ "Next, …" step), CallAction;
        a completed form → dispatches FormCompleted → FormSubmitter POSTs it in the background
     4. Repeat with a previous answer → that answer word for word; otherwise machine #2:
          form_information → Machine::form_informant(), calendar_help → Machine::calendar_refuser(),
          anything else → Machine::response_formulator(); extra context {backend_context, language}.
        On failure the call state is restored and the error returned.
     5. session.save_call_turn(CallTurn)
     → {"answer", "selected_function", "language_detected", "action", "reasoning"}
```

---

## File map

| path | holds |
|---|---|
| `app/src/vllm.rs` | `VllmClient` (`query`, `query_json`), `VllmError` (old client kept as `vllm.rs.old`) |
| `app/src/domain/machine.rs` | `Machine`, `query`, XML prompt rendering, `ValueSchema` / `OutputFormat` / `AllowedValue`, `ExtractedIntent`, `FormulatedResponse`, and the machines: `intent_matcher`, `response_formulator`, `calendar_refuser`, `form_informant` |
| `app/src/domain/call.rs` | `CallerIntent` (labels, `all`, `from_label`, `supported_requests`, `unsupported_backend_context`), `CallAction`, `GetInformationSupported` (+ `is_offered`), `FormSupported`, `flat_enum!` |
| `app/src/domain/form.rs` | `Form`, `FormField`, `FormFieldKind` (+ `parse`, `is_like`), `FormFieldValue`, `StepState`, form schemas in `FormSupported::build()`, test `every_form_builds` |
| `app/src/domain/information.rs` | `GetInformationSupported::fetch` (open-meteo weather, NBU EUR rate, fixed texts for the other two) |
| `app/src/domain/call_session.rs` | `CallSession`, slots, `CallState`, `CallMemory` / `CallTurn` / `Transcript`, `context()`, `last_answer()`, `CallLocks` / `CallLock` |
| `app/src/event/call_session.rs` | `CallSessionLoaded` + `InitialContextHandler` |
| `app/src/event/caller_intent.rs` | `IntentExtracted` + `CallStateHandler` (intent/form/step logic, `next_step`) |
| `app/src/event/form.rs` | `FormCompleted` + `FormSubmitter` |
| `app/src/event/events.rs` | handler registration, `events(&settings, &http)` |
| `app/src/event/event_bus.rs` | the dispatcher |
| `app/src/routes/call_session_middleware.rs` | the middleware above |
| `app/src/routes/api.rs` | routes, `handle_assistant_request`, machine #2 selection |
| `app/src/error.rs` | `ApiError` → `{"error": "..."}` with 400 / 502 / 504 |
| `client/index.html` | the mimic client, `include_str!`d into the binary (a change needs an image rebuild) |

---

## vLLM client (`vllm.rs`)

- `query(system, user) -> Result<String, VllmError>` returns the model's raw JSON text; `query_json` parses it.
- Sends `model`, `temperature`, `max_tokens`, `response_format: {"type": "json_object"}`, `messages: [system, user]`,
  and a bearer token only when `LLM_API_KEY` is set. Reads `choices[0].message.content`.
- `VllmError::Timeout` → **504**; `Transport`, `BadResponse`, `Malformed` → **502**.
- `[llm_settings]` in `app/config/settings.toml`: `timeout_seconds = 120`, `temperature = 0.0`, `max_tokens = 256`,
  shared by all machines. A cut-off answer shows up as `Malformed` (the old `Truncated` error is gone).

---

## Machines and prompt building (`domain/machine.rs`)

`Machine { role, rules }` and `query::<T>(llm, input, context, call_session)`. The system message:

```xml
<role>…</role>
<rules>
  <rule>Set `field` to: <valid_value_description>[. Use only a value listed in <allowed_values><field>]</rule>  ← per output field
  <rule>…the machine's own rules…</rule>
</rules>
<allowed_values><caller_intent><allowed_value>…</allowed_value>…</caller_intent></allowed_values>  ← strict fields only
<json_output_format>{ "machine_reasoning": "string", …, "form_field": "string", … }</json_output_format>
<conversation_history><call_turn>…</call_turn></conversation_history>  ← omitted when empty
<context>
  <current_time>…</current_time><form_state>{json}</form_state>      ← the session's context, sorted
  <backend_context>…</backend_context><language>en</language>        ← the turn's own context, sorted, LAST
</context>
```

The user message is `<utterance>…</utterance>`.

- **Context order:** the session's context first, then the `context` argument (which wins on a clash), each sorted.
  `backend_context` therefore sits after the long `form_state`, next to the utterance. Before this, machine #2 often
  ignored it and repeated its last confirmation question after a "yes".
- **Escaping:** `role` and `rules` are verbatim so they can name other sections by tag; everything else (utterance,
  history, context, allowed values) is escaped for `<`, `>` and `&`. A consequence: tags written inside
  `backend_context` text arrive as `&lt;…&gt;`, so backend texts must not refer to tags.
- **`<json_output_format>`** comes from schemars (inline subschemas), in struct order (`serde_json/preserve_order`).
  An `Option` field is shown with its own type (`"string"`), never as `["string", "null"]`: the model copied that as
  an array (`"form_field": ["null"]`), which failed to parse. Optional fields are to be *left out* when they have no
  value, and `#[serde(default)]` makes them `None`.
- **Logging:** every call logs `Machine prompt` (full system and user XML) and `Machine answered` at `info`.
- **Adding an output type:** a newtype per field implementing `ValueSchema`, a struct deriving
  `JsonSchema, Deserialize, Default` implementing `OutputFormat::iter_schemas` (keys = serde names). Allowed values
  come from `T::default()`, so they can't depend on the call's state yet.

### Machine #1: intent matcher → `ExtractedIntent`

| key | meaning |
|---|---|
| `machine_reasoning` | one sentence |
| `detected_language` | ISO 639-1 |
| `caller_intent` | strict: `CallerIntent::all()` labels, `unsupported` when nothing fits |
| `form_field` | optional: the `<form_state>` field the value is for; left out when there is none |
| `form_field_value` | optional: a `date` field as `YYYY-MM-DD`; a `spoken_date` field as the caller's words in lowercase English ("next saturday", "tomorrow evening") without working the date out; numbers as digits; yes/no as true/false; text as said |

The model writes `""` instead of leaving the keys out, so `form_field()` / `form_field_value()` treat `""` as absent.

Rules, in short: classify only the latest utterance (history for elliptical answers); `get_information[…]` /
`start_form[…]` only on an exact match; no form intents without `<form_state>`; `awaiting_confirmation` →
`confirm_yes` / `confirm_no` / `correct_form_field_value`; `queued` → `provide_form_field_value`; a new value for a
completed field → `correct_form_field_value`; "the same as before" → `refer_to_context_for_form_field_value`; a request
unrelated to the form is classified on its own; **greetings and small talk → `unsupported`**; **calendar questions →
`get_information[calendar_help]`**; **questions about the form or the values given → `get_information[form_information]`**;
when to use `repeat`, `end_call`, `transfer_to_human`.

### Machine #2: three variants, one output (`FormulatedResponse { spoken_response }`)

- `response_formulator()` — the default. Rules: reply in `<language>`; only facts and instructions from
  `<backend_context>`; one question at most; with no `<form_state>` after information or a completed/cancelled form,
  ask whether the caller needs anything else; plain words, no snake_case, no markdown. Its old rule "when
  `<form_state>` has a `current_field`, ask for it or read it back" was **removed**: it overrode the next step in
  `backend_context` (3 of 4 replayed turns fixed without it).
- `form_informant()` — for `form_information`: answers from `<form_state>` (no value = not given yet, not `completed` =
  not confirmed yet), never says the form is booked before every field is confirmed, then follows the next step.
- `calendar_refuser()` — for `calendar_help`: says sorry it can't help with dates, names no date or day, then follows
  the next step. The default formulator answered "what's the date of next Saturday" with a made-up date whatever
  `backend_context` said, even with an explicit "do not answer".

---

## Intents (`domain/call.rs`) and handling (`event/caller_intent.rs`)

| intent | label | handling |
|---|---|---|
| `Unsupported` | `unsupported` | `unsupported_backend_context()`: polite refusal listing the *offered* requests (`is_offered` hides `calendar_help` and `form_information`) |
| `GetInformation` | `get_information[get_current_weather_in_berlin]`, `…[get_current_uah_per_eur]` | fetched in `api.rs` (5 s timeout); failure → apologize |
| | `get_information[calendar_help]` | fixed text; answered by `calendar_refuser` |
| | `get_information[form_information]` | fixed text; answered by `form_informant` from `<form_state>` |
| `StartForm` | `start_form[doctor_appointment]` | `FormInProgress(form.build())`; same form running → continue it; another → replaced |
| `ProvideFormFieldValue`, `ReferToContextForFormFieldValue`, `CorrectFormFieldValue` | … | `fill` (below); no form → "nothing to fill" |
| `ConfirmYes` | `confirm_yes` | awaiting → completed; last field → form complete; queued bool → `true` |
| `ConfirmNo` | `confirm_no` | awaiting → value cleared, queued, counter +1; at 3 → offer a human; queued bool → `false` |
| `CancelForm` | `cancel_form` | state → `Idle` |
| `Repeat` | `repeat` | last answer word for word (no LLM) |
| `EndCall` | `end_call` | state → `Idle`, action `end_call` |
| `TransferToHuman` | `transfer_to_human` | action `transfer_to_human`, state kept |

`fill`: the target is the field named in `form_field` if it exists, else the current field. The value is parsed with
the field's kind; `fill_field` sets it `awaiting_confirmation` (a completed field is reopened and becomes current).
**A value for a field after the current one first confirms the current one** if it is awaiting confirmation: the
caller moved on from the value read back to them, which accepts it. (In the live test the name was read back without a
question and stayed unconfirmed for the rest of the form.)

**Next step:** while a form is open and the action is `continue`, the handler appends one sentence to
`backend_context`: `Next, ask the caller for: <description>.` or
`Next, ask the caller to confirm that <description> is <value>.` The per-case messages only state what happened.

---

## Forms (`domain/form.rs`)

- `Form { kind, fields }`, `FormField { name, description, kind, value, state, confirmation_failed_counter }`.
  A field is a step: `Queued` → `AwaitingConfirmation` → `Completed`; the current field is the first not completed.
- Methods: `field` (builder), `current_field`, `find_field`, `is_filled`, `is_ahead`, `fill_field`, `confirm_current`,
  `reject_current`, `values_summary`, `context_value` (the `form_state` JSON: `{"form", "current_field", "fields"}`).
- Kinds and `parse`: `string` (non-empty), `unsigned_integer`, `integer`, `float` (comma = decimal point), `bool`
  (true/yes, false/no), `date` (ISO `YYYY-MM-DD`, `time` crate), **`spoken_date`** (non-empty text kept as said, for
  the receiving project to work out). Values serialize externally tagged, e.g. `{"spoken_date": "next saturday"}`.
- **No two neighbouring steps may take the same kind of value** (`FormFieldKind::is_like`: both dates, both numbers,
  or equal): an answer meant for one fits the other, so the caller or the matcher can put it in the wrong field. The
  builder asserts it; `cargo test -p app --lib every_form_builds` builds every form so a bad order fails a test, not a
  call.
- `DoctorAppointment`: `patient_name` (string), `date_of_birth` (date), `reason` (string), `appointment_date`
  (spoken_date). Placeholder fields.

### Why relative dates are kept as said

Tested by replaying the logged prompt (today Saturday 2026-09-26) on 8 phrases: the 7B model got 1 right when asked
for `YYYY-MM-DD` ("next Saturday" → 09-30, a Wednesday), 0 of 6 with a 15-day calendar to look them up in, and still did
arithmetic when asked for relative words. Copying the caller's words in English worked for 6 of 8. So the appointment
date is stored as said; the receiver interprets it against `completed_at`. A read-back never names a computed date.

---

## Form completion → another project

- When the last field is confirmed, `CallStateHandler` logs `Form completed`, sets `Idle` and dispatches
  `FormCompleted { call_id, form }`. `FormSubmitter` POSTs, in a spawned task (10 s timeout), to `FORM_SUBMIT_URL`:
  ```json
  {"call_id": "…", "completed_at": "2026-09-26T15:51:02.55+03:00", "form": {"kind": "doctor_appointment", "fields": [ … ]}}
  ```
  Logs `Form submitted` or `Could not submit the form`; with no URL it logs a warning and sends nothing.
- `FORM_SUBMIT_URL` is in `.env` (empty for now) and passed by `docker-compose.yml`. From the container, a service on
  the host is `http://host.docker.internal:<port>/…`. Verified with a throwaway receiver.
- No retry. `call_id` is in the payload so the receiver can deduplicate.

---

## Call session and storage (`domain/call_session.rs`)

- Redis key `call_id.<id>`: `slots` as MessagePack, TTL `call_session_ttl_seconds` (3600), saved on every request.
  Slots `call_memory` (`CallMemory`) and `call_state` (`CallState`: `Idle` | `FormInProgress(Form)`) are JSON strings;
  a malformed slot logs an error and falls back to the default.
- `initial_context` isn't persisted: `CallSessionLoaded` rebuilds `current_time` every request, e.g.
  `"Saturday, 2026-09-26 15:25 +03:00"`, in the container's clock (`TZ` from `APP_TIMEZONE`).
- `context()` = `initial_context` + `form_state` while a form is active.
- **Per-call lock:** `AppState.call_locks` (`CallLocks`) gives one `tokio::sync::Mutex` per call id. The middleware
  takes it before loading and holds it until after saving, so the next turn of the call loads what this one saved;
  other calls are not blocked. Entries are removed once no turn holds or waits for them. In-process only. Verified with
  concurrent requests (second turn started 1 ms after the first finished and saw its history).

## Event system (`event/`)

Handlers are synchronous (`EventHandler<E>::handle(&self, &mut E, &Dispatcher)`); events own their data; I/O happens
before dispatch, or is spawned (form submission). Handlers may dispatch nested events. Registered:
`CallSessionLoaded` → `InitialContextHandler`, `IntentExtracted` → `CallStateHandler`, `FormCompleted` → `FormSubmitter`.

## HTTP API

- `POST /assistant/handle-request` (needs `x-call-id`), `GET /mimic-client`, `GET /version`, `GET /test-session`
  (still panics: it expects a `UserSession` nothing adds; left alone on purpose). Only the assistant route is behind
  the call-session middleware, so the page, `/version` and CORS preflights work without the header.
- Response: `{"answer", "selected_function", "language_detected", "action": "continue"|"end_call"|"transfer_to_human", "reasoning"}`.
  `language` in the request is only logged. Errors: `{"error"}` with 400 / 504 / 502.

## Mimic client (`client/index.html`)

- One call id per page load (`crypto.randomUUID()`), shown on the page and sent as `x-call-id`; reload = new call.
- Acts on `action` after the answer is read out: `end_call` hangs up, `transfer_to_human` too (nothing to transfer to).
- The microphone is off from sending an utterance until the answer has been read out (or failed). Typed input goes
  through the same path. Badges: `unsupported` red, `transfer_to_human` amber.

---

## Configuration and environment

- `.env` / `.env.dist`: `APP_TIMEZONE=Europe/Kyiv` (compose passes it as `TZ`; the runtime image installs `tzdata`),
  `FORM_SUBMIT_URL=`.
- **GPU memory:** `LLM_GPU_MEMORY_UTILIZATION=0.80`. At 0.90 vLLM left the desktop too little of the 12 GB; opening a
  browser page then made Windows page vLLM's memory out and generation fell to <1 token/s (3-minute turns). A change
  only applies when the container is recreated (`docker compose up -d vllm`), not on a restart.
- Cargo builds on this machine go to the shared `H:/cargo-target` (global `~/.cargo/config.toml`); a scratch copy of
  this workspace built there overwrites this tree's artifacts. Use a separate `CARGO_TARGET_DIR` for scratch builds.

## How to run

```bash
./update-app.sh                          # builds the app image, then recreates only the app container
docker compose up -d --build app         # the same, less careful
curl -s -X POST http://localhost:8080/assistant/handle-request -H 'Content-Type: application/json' \
  -H 'x-call-id: test-call-1' -d '{"request_text": "I would like to book a doctor appointment"}'
```

- Mimic client: http://localhost:8080/mimic-client
- Logs: `./logs/app.log.<date>` — search `Machine prompt`, `Machine answered`, `Form completed`, `Form submitted`.
- `cargo check -p app --all-targets`; `cargo test -p app --lib every_form_builds` (the other 5 tests need Postgres/Redis).
- Local `cargo run` needs `localhost` URLs (`.env.test` has them).

## Verification (2026-09-26)

- Live runs against vLLM + Redis in Docker: a 15-turn doctor-appointment conversation with a greeting, a name, a
  date-of-birth correction, a form question, a calendar question, a spoken appointment date, "did you book it", a
  completion and a goodbye all gave the intended intent, state and reply.
- Prompt changes were chosen by replaying logged prompts against vLLM with variations (scripts were throwaway).

## What the 7B model does badly (learned by replay)

- Any date arithmetic, with or without a calendar in the prompt.
- Follows `backend_context` poorly when it is not the last thing in `<context>`; repeats its previous question.
- Answers a question it should refuse (a made-up date) unless a dedicated variant's *role* says to refuse.
- Copies type hints literally (`["string", "null"]` → arrays) and writes `""` instead of leaving keys out.
- Takes a missing year from other dates in the history ("12th of October" → 1999-10-12) and follows examples in rules
  too eagerly (a "june 12th" example made it write a date of birth as "june 12th").

---

## Known gaps and next steps

**Dialogue**
1. After a form is completed there is no `form_state`, so `form_information` can't answer "what did you book?".
2. `form_informant` sometimes answers without asking the next step's question.
3. "What day is it today" is still answered (correctly, from `current_time`) by `calendar_refuser`.
4. No record of the pending question in `CallState`; a yes/no to "anything else?" depends on the history.
5. Allowed intents don't depend on state (`confirm_*` offered with nothing pending).
6. One value per turn; no `required` flag; no skip / don't know / unclear (speech-recognition noise) intents; no
   decision on a `GeneralQuestion` intent.
7. `EndCall` / `TransferToHuman` only set `action`.

**Correctness and safety**
8. PII in logs: full prompts with history and form values at `info`, and the completed form in `Form completed`.
9. Unbounded history in both prompts and Redis (a rolling window of 4–6 turns was suggested).
10. `x-call-id` is trusted as given: ids must be unguessable or authenticated by the telephony side.
11. The call lock is per process; several app instances would race (last write wins).
12. Form submission isn't retried; a failed POST is only logged.
13. No truncation detection (`finish_reason == "length"` shows up as `Malformed`).

**Leftovers**
14. Prompt-file remnants: `llm_system_prompt_file` in `settings.toml`/`settings.rs`, `LLM_SYSTEM_PROMPT_FILE` and the
    `./app/prompts` mount in `docker-compose.yml`, `app/prompts/` (README describes the old contract; the `.txt` files
    are reference only).
15. `routes/middleware.rs` (`session_middleware`) and `/test-session` unused/broken, `PROMPT_LOOP_FLAG` in `main.rs`,
    `app/src/vllm.rs.old`, `bash.exe.stackdump`, many unused imports, `sqlx::migrate!` commented out.

---

## Still true from 2026-09-17

- Logging: stdout plus daily rolling files via `tracing-appender` in `LOG_DIR` (`/var/log/app` → `./logs`); filter
  `RUST_LOG=info,sqlx=warn,tower_http=info`; settings load before logging starts.
- LLM settings: 120 s timeout because a cold/contended GPU took 30 s+ (a timeout is its own 504); `max_tokens` capped
  because an uncapped repetition loop ran to the 4096-token context.
- `serde_json` `preserve_order` is enabled workspace-wide.
- Earlier research (`git show fd90a3d:context.md`): record where a slot value came from; idempotent commits; hand off
  after N failed turns; order prompt layers static → volatile (helps vLLM's prefix cache).

## Working agreements

- Keep it stupid simple; do only what the current step asks.
- Follow the existing style: a newtype + `ValueSchema` per machine-output field, `/* --- */` separators, few comments.
- The user commits; edits they make between steps are deliberate.
- When the tree doesn't compile for unrelated reasons, verify in a scratch copy (with its own `CARGO_TARGET_DIR`).
