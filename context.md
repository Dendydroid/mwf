# Context — ai_assistant

**Updated:** 2026-09-26 · **Repo:** `H:\rust\ai_assistant` (branch `master`, last commit `b524d2d ver 1`)
**Stack:** Rust / axum 0.7 · Redis (call sessions) · Postgres (connected, not used by the assistant) ·
vLLM serving `Qwen/Qwen2.5-7B-Instruct-AWQ` over the OpenAI-compatible API
**Reference project:** `H:\rust\ai_autocalls_rust` (earlier, larger implementation) ·
**Background:** `docs/call-assistant-handbook.html` ("The Call Assistant Engineering Handbook", 2026-09-17)

This replaces the 2026-09-17 handoff, which described the previous single-prompt design
(`SystemPrompt`, `ActionDecision`, prompt hot-reload, `humanlike_sentence_answer`). None of that
exists any more. The old document, including its research notes on multi-step forms, is in git:
`git show fd90a3d:context.md`.

---

## What it is

The backend of a phone assistant. Each caller utterance is one HTTP request, tied to a call by the
`x-call-id` header. Every turn runs two LLM "machines":

1. **Intent matcher** classifies the utterance into a `CallerIntent` and extracts a form value.
2. **Response formulator** writes the sentence the caller hears.

Between them, deterministic Rust code changes the call's state (forms and their steps) through the
event system. **The model extracts, code decides.**

It currently supports two information requests (current weather in Berlin, EUR→UAH rate) and one
form (doctor appointment), filled field by field with each value confirmed by the caller. It also
handles repeat, end call and transfer to a human, and gives a polite refusal listing what is
supported.

---

## One turn, end to end

```
POST /assistant/handle-request   header x-call-id: <id>   body {"request_text": "...", "language": "en-US"?}
│
├─ call_session_middleware (routes/call_session_middleware.rs), global layer in main.rs
│    missing/empty x-call-id → 400 {"error": "x-call-id header is required"}
│    CallSession::from_or_new(call_id)                       Redis key call_id.<id>
│    dispatch CallSessionLoaded → InitialContextHandler fills initial_context (current_time)
│    request extension: Arc<RwLock<CallSession>>
│    after the handler: save_slots(call_session_ttl_seconds = 3600) on EVERY request → sliding TTL
│
└─ handle_assistant_request (routes/api.rs), holds the session write lock for the whole turn
     1. Machine::intent_matcher().query::<ExtractedIntent>(…)   unknown intent label → Unsupported (warn)
     2. GetInformation → selected.fetch(&http_client)             done here: event handlers cannot await
     3. IntentExtracted { intent, form_field, form_field_value, information, state (moved in) }
          → CallStateHandler changes CallState, sets backend_context + CallAction; state moved back
     4. Repeat with a previous answer → that answer word for word, no LLM call
        otherwise Machine::response_formulator().query::<FormulatedResponse>(…) with extra context
        { backend_context, language }; on failure the call state is restored and the error returned
     5. session.save_call_turn(CallTurn) → call memory
     → {"answer", "selected_function", "language_detected", "action", "reasoning"}
```

---

## File map

| path | holds |
|---|---|
| `app/src/vllm.rs` | `VllmClient` (`query`, `query_json`), `VllmError`. Rewritten from scratch this session; the old client is kept as `app/src/vllm.rs.old` |
| `app/src/domain/machine.rs` | `Machine`, `query`, XML prompt rendering, `ValueSchema` / `OutputFormat` / `AllowedValue`, output types `ExtractedIntent` (#1) and `FormulatedResponse` (#2), and each machine's role and rules (`Machine::intent_matcher()`, `Machine::response_formulator()`) |
| `app/src/domain/call.rs` | `CallerIntent` (labels, `all()`, `from_label`, `supported_requests`, `unsupported_backend_context`), `CallAction`, `GetInformationSupported`, `FormSupported`, `flat_enum!` macro |
| `app/src/domain/form.rs` | `Form`, `FormField`, `FormFieldKind` (+ `parse`), `FormFieldValue` (+ `Display`), `StepState`, form schemas in `FormSupported::build()` |
| `app/src/domain/information.rs` | `GetInformationSupported::fetch` (open-meteo weather, National Bank of Ukraine EUR rate) |
| `app/src/domain/call_session.rs` | `CallSession`, slots, `CallState`, `CallMemory` / `CallTurn` / `Transcript`, `context()`, `last_answer()` |
| `app/src/event/call_session.rs` | `CallSessionLoaded` event + `InitialContextHandler` |
| `app/src/event/caller_intent.rs` | `IntentExtracted` event + `CallStateHandler` (all intent/form/step logic) |
| `app/src/event/events.rs` | handler registration (`events()` → `Dispatcher`, held in `AppState`) |
| `app/src/event/event_bus.rs` | the dispatcher (pre-existing) |
| `app/src/routes/call_session_middleware.rs` | the middleware above |
| `app/src/routes/api.rs` | routes and `handle_assistant_request` |
| `app/src/error.rs` | `ApiError` → `{"error": "..."}` with 400 / 502 / 504 (pre-existing) |

---

## vLLM client (`vllm.rs`)

- `query(system: &str, user: &str) -> Result<String, VllmError>` returns the model's raw JSON text.
  `query_json` is the same, parsed into a `serde_json::Value`.
- The request sends `model`, `temperature`, `max_tokens`, `response_format: {"type": "json_object"}`
  and `messages: [system, user]`, plus a bearer token only when `LLM_API_KEY` is set. The answer is read
  from `choices[0].message.content`.
- `VllmError`:
  - `Timeout { budget_seconds }` → **504**
  - `Transport`, `BadResponse(Value)` and `Malformed { source, content }` → **502**
- Settings (`app/config/settings.toml`, `[llm_settings]`) are `timeout_seconds = 120`,
  `temperature = 0.0` and `max_tokens = 256`. The same cap applies to both machines.
- The old client had a `Truncated` error for answers cut off at `max_tokens`
  (`finish_reason == "length"`). It was dropped in the rewrite, so a cut-off answer now shows up as
  `Malformed`.

---

## Machine and prompt building (`domain/machine.rs`)

`Machine { role, rules }` and:

```rust
query<T: OutputFormat + Default + JsonSchema + DeserializeOwned>(
    &self, llm: &VllmClient, input: &str, context: &HashMap<String, String>, call_session: &CallSession,
) -> Result<T, VllmError>
```

- The context is `call_session.context()` plus the `context` argument; the argument wins on a
  clash. History comes from `call_session.get_conversation()`.
- The prompt is built with **quick-xml 0.42** (`serialize` feature) and indented by 2. The system
  message has this layout:

```xml
<role>…</role>
<rules>
  <rule>Set `field` to: <valid_value_description>[. Use only a value listed in <allowed_values><field>]</rule>  ← one per output field
  <rule>…the machine's own rules…</rule>
</rules>
<allowed_values><caller_intent><allowed_value>…</allowed_value>…</caller_intent></allowed_values>  ← only fields with strict values
<json_output_format>{ "machine_reasoning": "string", …, "form_field": ["string", "null"] }</json_output_format>
<conversation_history><call_turn>…serialized CallTurn…</call_turn></conversation_history>  ← omitted when empty
<context><backend_context>…</backend_context><current_time>…</current_time><form_state>{json}</form_state><language>en</language></context>  ← keys sorted, omitted when empty
```

  The user message is `<utterance>…</utterance>`.
- **Escaping:** `role` and `rules` are written verbatim so they can refer to other sections by tag.
  Everything else (utterance, history, context, allowed values) is escaped for `<`, `>` and `&`, so
  a caller can't close a tag.
- `<json_output_format>` is generated with **schemars** (inline subschemas) as a map from field name
  to JSON type. The workspace's `serde_json/preserve_order` keeps it in struct order.
- **Logging:** every call logs `Machine prompt` (the full system and user XML) and `Machine answered`
  (the raw content) at `info`, with `call_id` and the output type name.
- **Adding a machine output type:**
  1. Write one newtype per field implementing `ValueSchema` (`valid_value_description`, and
     optionally `allowed_values`, which makes it strict).
  2. Write a struct deriving `JsonSchema, Deserialize, Default` that implements
     `OutputFormat::iter_schemas`. It returns `(json key, &dyn ValueSchema)`, and the keys must match
     the serde names.
  3. Allowed values come from `T::default()`, so they can't depend on the call's state yet.

### Machine #1: intent matcher → `ExtractedIntent`

| JSON key | meaning |
|---|---|
| `machine_reasoning` | one-sentence analysis |
| `detected_language` | ISO 639-1 code |
| `caller_intent` | strict: every label of `CallerIntent::all()`; `unsupported` when nothing fits |
| `form_field` | optional: the `<form_state>` field the value is for |
| `form_field_value` | optional, normalized: dates `YYYY-MM-DD` (relative dates resolved from `current_time`), numbers as digits, yes/no as `true`/`false` |

The last two use `#[serde(default)]`, so they may be missing from the answer. Its rules cover:
- Classify only the latest utterance, using the history for short answers.
- `get_information[…]` and `start_form[…]` require an exact match; anything similar but different is
  `unsupported`.
- No form intents when there's no `<form_state>`.
- When the current field is `awaiting_confirmation`, answers are `confirm_yes`, `confirm_no` or
  `correct_form_field_value`.
- When the current field is `queued`, an answer (including yes/no for a `bool` field) is
  `provide_form_field_value`.
- A new value for a completed field is `correct_form_field_value` with `form_field` set.
- "Same as before" is `refer_to_context_for_form_field_value`, resolved from the history.
- A request unrelated to the form is classified on its own.
- When to use `repeat`, `end_call` and `transfer_to_human`.

### Machine #2: response formulator → `FormulatedResponse`

It has one key, `spoken_response`: 1–2 sentences of plain speech, with numbers and dates written
the way they're spoken. Its rules:
- Reply in `<language>`.
- Use only facts from `<backend_context>`.
- If `<form_state>` has a `current_field`, end with exactly one question: ask for it when it's
  `queued`, or read the value back and ask for confirmation when it's `awaiting_confirmation`. Use
  the field's `description`.
- Never ask more than one question.
- With no form active, after information, a completed form or a cancelled one, ask whether the
  caller needs anything else.
- No snake_case names, no markdown or emojis.

---

## Intents (`domain/call.rs`) and how they are handled (`event/caller_intent.rs`)

Labels come from `Display`. `CallerIntent::all()` is the allowed-value list, protected by an
exhaustive `match` so a new variant can't be forgotten. `from_label` parses the model's answer.

| intent | label | handling |
|---|---|---|
| `Unsupported` | `unsupported` | `backend_context` is `unsupported_backend_context()`: a polite refusal that lists every `GetInformation` and `StartForm` label "in plain words". It replaced `SmallTalk`, so greetings land here too |
| `GetInformation { selected }` | `get_information[get_current_weather_in_berlin]`, `get_information[get_current_uah_per_eur]` | facts fetched in `api.rs` (5 s timeout); if the fetch fails → apologize |
| `StartForm { form }` | `start_form[doctor_appointment]` | `FormInProgress(form.build())`. If the same form is running → continue it; a different one is replaced |
| `ProvideFormFieldValue`, `ReferToContextForFormFieldValue`, `CorrectFormFieldValue` | `provide_form_field_value`, `refer_to_context_for_form_field_value`, `correct_form_field_value` | one shared path, `fill`, described below. With no form → "nothing to fill" |
| `ConfirmYes` | `confirm_yes` | `awaiting_confirmation` → `completed`; if that was the last field the form is complete (summary in `backend_context`, logged as `Form completed`, state → `Idle`). A `queued` bool field → value `true`. With no form → "ask what they need" |
| `ConfirmNo` | `confirm_no` | `awaiting_confirmation` → value cleared, back to `queued`, counter +1; at **3** rejections → also offer a human. A `queued` bool field → `false`. With no form → "ask how you can help" |
| `CancelForm` | `cancel_form` | state → `Idle` |
| `Repeat` | `repeat` | `api.rs` replays the last answer word for word; with no previous answer, machine #2 handles it |
| `EndCall` | `end_call` | state → `Idle`, action `end_call`, short goodbye |
| `TransferToHuman` | `transfer_to_human` | action `transfer_to_human`, state kept |

How `fill` works:
1. The target is the field named in `form_field` if it exists, otherwise the current field.
2. The value is parsed with that field's kind and stored with `fill_field`, which sets it to
   `awaiting_confirmation`. A completed field is reopened this way and becomes current.
3. A missing or unparseable value → ask for it again.

A non-form intent in the middle of a form leaves the form untouched, and machine #2's rule adds the
current field's question at the end of the reply.

---

## Forms and steps (`domain/form.rs`)

- `Form { kind: FormSupported, fields: Vec<FormField> }`.
- `FormField { name, description, kind: FormFieldKind, value: Option<FormFieldValue>, state: StepState, confirmation_failed_counter }`.
- **A field is a step.** `StepState` goes `Queued` → `AwaitingConfirmation` → `Completed`. The
  current field is the first one that isn't `Completed`, and `is_filled()` means none is left.
  Nothing else is stored; your draft's borrowing `Step<'a>` couldn't be cached, so it was merged
  into `FormField`.
- Methods: `new`/`field` (builder), `current_field`, `find_field`, `is_filled`, `fill_field`,
  `confirm_current`, `reject_current`, `values_summary`, and `context_value`. The last one gives the
  JSON used as `form_state`: `{"form", "current_field", "fields": […]}`.
- **Schemas:** `FormSupported::build()` matches on the variant. `DoctorAppointment` has
  `patient_name` (String), `date_of_birth` (Date), `appointment_date` (Date) and `reason` (String).
  These fields are placeholders.
- `FormFieldKind::parse`:
  - String: must not be empty
  - `u64` / `i64`
  - `f64`: a comma is read as a decimal point
  - bool: `true`/`yes` and `false`/`no`
  - Date: ISO 8601 `YYYY-MM-DD` (`time`)
- `FormFieldValue` is serialized as snake_case, externally tagged, e.g. `{"date": "1990-01-31"}`. This
  relies on the `time` crate's `serde-human-readable` feature.
- A completed form is logged and the state goes back to `Idle`. **It isn't submitted anywhere yet.**

---

## Call session and storage (`domain/call_session.rs`)

- The Redis key is `call_id.<id>`. The value is `slots: HashMap<String, SlotValue>` encoded as
  MessagePack (`rmp-serde`), with a TTL of `call_session_ttl_seconds` (3600). It's saved on every
  request, so the TTL counts from the last turn.
- Slots hold JSON strings (`SlotValue::JsonObject`):
  - `call_memory` holds `CallMemory { conversation: Vec<CallTurn> }`. It's written by `save_call_turn`.
  - `call_state` holds `CallState` (`Idle` | `FormInProgress(Form)`). It's written in `save_slots`, so
    changes made in place are saved too.
  - Both are read by `read_json_slot`. A malformed slot logs an error and falls back to the default.
- `initial_context` isn't persisted. `CallSessionLoaded` handlers rebuild it on every request. Today
  that's only `current_time`, e.g. `"Saturday, 2026-09-26 10:00 +03:00"`, in the server's local time
  via chrono.
- `context()` returns `initial_context` plus `form_state` while a form is active.
- The session's `RwLock` only serializes the turns of one call within a single process. With several
  app instances, the last write wins.

## Event system (`event/`)

- Handlers are synchronous: `EventHandler<E>::handle(&self, &mut E, &Dispatcher)`. Events must be
  `Send + 'static`, so they own their data. The call state is `mem::take`n into `IntentExtracted` and
  moved back out after dispatch.
- Anything that needs I/O (the information fetch) happens before dispatch.
- Registered handlers: `CallSessionLoaded` → `InitialContextHandler` and `IntentExtracted` →
  `CallStateHandler`.

## HTTP API

- `POST /assistant/handle-request` takes the `x-call-id` header and `{request_text, language?}`.
  `language` is only logged; the language used is the one machine #1 detects, with `en` as fallback.
- The response looks like this:
  ```json
  {"answer": "…", "selected_function": "start_form[doctor_appointment]", "language_detected": "en",
   "action": "continue" | "end_call" | "transfer_to_human", "reasoning": "…"}
  ```
  The mimic client leads its badge row with `selected_function` and speaks in `language_detected`.
- Errors come back as `{"error": "..."}`:
  - 400: empty text or missing header
  - 504: vLLM timeout
  - 502: any other inference failure
- Other routes: `GET /mimic-client` (`client/index.html`), `GET /version`, `GET /test-session`. All
  of them are behind the call-session middleware too.

---

## Done this session (2026-09-26), in order

1. **`vllm.rs` rewritten from scratch**, from 477 lines to about 100. It now takes plain strings and
   returns JSON text. Removed: `SystemPrompt` (the file-backed prompt that reloaded every turn),
   `ActionDecision`, `classify`, the per-instruction answer prompts, and the request/response
   structs.
2. **`Machine::query` and the XML prompt builder.** `iter_schemas` now yields `(key, schema)`,
   `ExtractedIntent` derives `Default`, and `AllowedValue` implements `Display`.
3. `<context>` was moved after `<conversation_history>`.
4. **Output types for machine #2** (`SpokenResponse`, `FormulatedResponse`), adapted from
   `app/prompts/system_prompt_response_formulator.txt`.
5. **`call_session_middleware`** replaced `session_middleware` in `main.rs`. The header was
   `call_id` at first; you renamed it to `x-call-id`.
6. **Forms and steps:**
   - `form.rs` rebuilt from your draft types.
   - `FormSupported::build()` added.
   - `CallState::FormInProgress(Form)` is saved in the `call_state` slot.
   - `CallSession::context()` added.
   - `CallSessionLoaded` + `InitialContextHandler` added. You chose an event handler over a plain
     function.
   - `FormSupported` now derives serde, and `time` gained `serde-human-readable`.
7. **Intent review:**
   - `SmallTalk` → `Unsupported`.
   - `Repeat`, `EndCall` and `TransferToHuman` added.
   - `supported_requests()` and `unsupported_backend_context()` added.
   - The intent field's description now names `unsupported` as the fallback.
8. **Handler rewrite:**
   - `Machine::query` takes `&CallSession`, and form state reaches `<context>`.
   - Roles and rules defined for both machines; `ExtractedIntent` gained `form_field` and
     `form_field_value`.
   - `IntentExtracted` + `CallStateHandler` added, and fetchers in `information.rs`. The weather call
     was restored from `git show 143158c:app/src/domain/instructions.rs`.
   - Added `FormFieldKind::parse`, `Form::fill_field` (it replaced `fill_current`), `find_field`,
     `values_summary`, `CallAction`, `CallerIntent::from_label` and `CallSession::last_answer`.
   - Prompt logging.

New dependencies: `quick-xml = { version = "0.42", features = ["serialize"] }`, and
`time = { version = "0.3", features = ["serde-human-readable"] }`. `schemars` 1.2.2 was already
present and is now used.

## Verification

- `cargo check -p app --all-targets` compiles: both the library and the binary.
- Tested with throwaway tests in a scratch copy; **none of them are committed**:
  - A full doctor-appointment conversation through the real dispatcher. Provide, reject, an
    unparseable date, correcting an earlier field (which reopens it and then comes back), completion,
    cancel, unsupported and end call each gave the expected state and `backend_context`.
  - Both machines' prompts rendered with form state and history.
  - Escaping: `</utterance>` typed by a caller comes out as `&lt;/utterance&gt;`.
  - Both fetchers returned live data, e.g. `1 EUR = 51.1358 UAH … for 28.09.2026`.
- **Never run end to end against vLLM and Redis**, because Docker wasn't running. The machines'
  rules haven't been tuned against the real model yet.
- The only tests in the repo are the 5 older ones in `lib.rs`, which need Postgres and Redis.

---

## Known gaps and next steps

**Blocking a first live test**
1. `client/index.html` doesn't send `x-call-id`, so every request gets a 400. It needs a call id
   generated per page load and sent in that header.
2. The middleware wraps every route. `GET /mimic-client` and `GET /version` need the header too, so a
   browser can't load the page. Either apply the middleware to the assistant route only, or exempt
   those routes.
3. `/test-session` panics: it expects a `UserSession` extension, which nothing adds any more.
   `routes/middleware.rs` (`session_middleware`) is unused.

**Correctness and safety**
4. **PII in logs:** full prompts, including history and form values (names, dates of birth), are
   logged at `info` into `./logs`. The old handoff already flagged `request_text` for the same
   reason (GDPR).
5. **Unbounded history:** every past turn goes into both prompts and into Redis. The old research
   suggested a rolling window of 4–6 turns.
6. **`x-call-id` is trusted as given:** anyone who knows a call id can read or change that call. Ids
   must be unguessable, or authenticated by the telephony side.
7. **Truncation detection is gone** (see the vLLM section).

**Dialogue design, discussed but not built**
8. No record of the pending question in `CallState`. A yes or no to "anything else?" depends on the
   model reading the history.
9. Allowed intents don't depend on state: `confirm_*` is offered even when nothing is pending.
10. One value per turn: "book for John on Tuesday" loses the extra values. A value given for a later
    field is stored as awaiting confirmation.
11. No `required` flag on fields, and no `SkipFormField` / `DontKnow` or `Unclear` (speech-recognition
    noise) intents. There's also no decision on a `GeneralQuestion` intent (open questions answered
    from the model's own knowledge).
12. `EndCall` and `TransferToHuman` only set `action`; nothing hangs up or transfers.
13. Completed forms go nowhere: nothing persists, books or deduplicates them (commits should be
    idempotent per call id).

**Leftovers to clean up**
14. Remnants of the old prompt-file design, none of them used by the code any more:
    - the `llm_system_prompt_file` setting (pointing at the deleted `prompts/system_prompt.txt`)
    - the `LLM_SYSTEM_PROMPT_FILE` variable and the `./app/prompts` mount in `docker-compose.yml`
    - `app/prompts/README.md`, which describes the old `humanlike_sentence_answer` contract
    - the `.txt` prompts in `app/prompts/`, which are reference only
15. Unused `PROMPT_LOOP_FLAG` in `main.rs`, `app/src/vllm.rs.old`, `bash.exe.stackdump` in the repo
    root, many unused imports, and `sqlx::migrate!` still commented out in `main.rs`.

---

## Still true from the 2026-09-17 session

- **Logging:** stdout plus daily rolling files via `tracing-appender` in `LOG_DIR` (`/var/log/app` in
  the container, mapped to `./logs`). The filter is `RUST_LOG=info,sqlx=warn,tower_http=info`.
  Settings load before logging starts, because `LOG_DIR` decides where logs go.
- **Why the LLM settings are what they are:** a 30 s timeout used to turn a slow, cold GPU into 502s,
  so the timeout is now 120 s and a timeout is its own 504 (worth retrying). `max_tokens` is capped
  because an uncapped repetition loop ran to the 4096-token context.
- **`serde_json` `preserve_order`** is enabled workspace-wide.
- **Earlier research** (full version in `git show fd90a3d:context.md`):
  - Slots should record where a value came from: verified by an API, or collected from the caller.
  - Commits should be idempotent.
  - Count failed and silent turns, and hand off to a human after N. The 3-rejections rule is a start.
  - Keep prompt layers ordered from static to volatile, which helps vLLM's prefix cache.

## How to run

```bash
docker compose up -d --build app          # db, redis and vllm must be healthy
curl -s -X POST http://localhost:8080/assistant/handle-request \
  -H 'Content-Type: application/json' -H 'x-call-id: test-call-1' \
  -d '{"request_text": "I would like to book a doctor appointment"}'
```

- Running locally with cargo needs `localhost` URLs. `.env` points at the compose hostnames, while
  `.env.test` already uses `localhost`.
- In `./logs/app.log.<date>`, search for `Machine prompt`, `Machine answered` and `Form completed`.
- `cargo check -p app --all-targets` should pass.

## Working agreements

- Keep it stupid simple, and do only what the current step asks ("only this for now").
- Follow the existing style: a newtype plus a `ValueSchema` implementation per machine-output field,
  `/* --- */` separators between sections, and few comments.
- You commit the work yourself. Edits you make between steps are deliberate, so build on them.
- When the tree doesn't compile for unrelated reasons, verify in a scratch copy instead of changing
  files outside the task.
