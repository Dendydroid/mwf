# CLAUDE.md — ai_autocalls (phone assistant backend)

Rust (axum 0.7) backend of a **phone assistant**. Every caller utterance (already transcribed) is one HTTP request tied
to a call by the `x-call-id` header. A local LLM (Qwen2.5-7B-Instruct, OpenAI-compatible API) **extracts values and
picks an intent; Rust code decides what happens and, inside a form, words the reply itself**. Redis holds call
sessions; Postgres is connected but unused by the assistant; lingua detects the caller's language.

Supported: current weather in Berlin, EUR→UAH rate, booking a doctor's appointment (a form filled field by field,
every value confirmed and validated, several values per answer), questions about the form in progress and about the
one filled out last, a calendar refusal, repeat, end call, transfer to a human, a greeting answered with what is
supported, and a polite refusal listing what is supported. A completed form is POSTed to another project. Languages: **German (default), English, Russian, Ukrainian**.

This file is the single project doc: how to run it, what a caller can do, how the code works, why it is built this
way (measured), how it is tested, and what is still open.

---

## 1. Run it

### macOS (Apple Silicon): this machine
No GPU in Docker on a Mac and the vLLM image is CUDA-only, so the model runs **natively in Ollama** and the app
container calls it on the host.

```bash
ollama serve &                         # or the Ollama app; must be up before the app takes calls
ollama pull qwen2.5:7b-instruct        # once, ~4.7 GB, 4-bit like the AWQ model
./update.sh                            # docker compose up -d --build --force-recreate  → db, redis, app
open http://localhost:8080/mimic-client
```

- `docker-compose.override.yml` (picked up automatically) puts `vllm` behind the `gpu` profile so it does not start,
  drops the app's dependency on it and sets `LLM_URL=http://host.docker.internal:11434/v1`.
- `.env` has `LLM_MODEL=qwen2.5:7b-instruct` (the Ollama name); `.env.dist` keeps the vLLM default.
- Ollama's OpenAI endpoint supports what the app sends: strict `response_format: json_schema` and `logprobs`. It does
  **not** reject a prompt longer than the context the way vLLM does (it answers it), so `ContextWindowFull` never
  happens here.
- **Build gotcha:** the `Dockerfile` does `COPY --from=builder /workspace/app/prompts /app/prompts` and compose mounts
  `./app/prompts`. The code reads no prompt file any more, but the build fails without the folder:
  `mkdir -p app/prompts && touch app/prompts/.gitkeep`.

### Linux / Windows+WSL2 with an NVIDIA GPU: the original setup
Remove `docker-compose.override.yml`, set `LLM_MODEL=Qwen/Qwen2.5-7B-Instruct-AWQ`, `./update.sh`. vLLM is
`docker/vllm` (its `entrypoint.sh` turns `LLM_*` env into `vllm serve` flags; `LLM_EXTRA_ARGS` for anything else).
Developed on an RTX 3080 Ti (12 GB) that also drives the Windows desktop: at `LLM_GPU_MEMORY_UTILIZATION=0.90` the
desktop starved and generation fell under 1 token/s, so use 0.80 there. A change applies only on
`docker compose up -d vllm` (recreate), not a restart. Bigger models (Qwen3.5-9B AWQ, Gemma 3/4 12B, Qwen3-14B) do not
fit 12 GB; Qwen3-8B-AWQ fits (`--max-model-len 8192`, `chat_template_kwargs: {"enable_thinking": false}`) but was
worse with the old prompts (218 vs 167 flagged replies of 580) and equal with the current design.

### Other ways
```bash
./update-app.sh [-f]          # rebuild + recreate ONLY the app container (db/redis/vllm untouched; -p git pull)
cargo run -- --prompt-loop    # no HTTP: one call id per process; reads .env, then .env.stdin, and settings.toml.
                              # a piped script must end with `exit` (it keeps reading empty lines at EOF)
cd app && set -a && . ../.env.stdin && set +a && APP_PORT=8089 cargo run     # HTTP server against local services
curl -s -X POST localhost:8080/assistant/handle-request -H 'Content-Type: application/json' \
  -H 'x-call-id: test-1' -d '{"request_text": "I would like to book a doctor appointment", "language": "en"}'
```

- **Logs:** `./logs/app.log.<date>` (UTC, bind-mounted from `/var/log/app`; stdout too). Filter
  `RUST_LOG=info,sqlx=warn,tower_http=info`. Grep `Machine prompt` (full system + user XML of every model call),
  `Machine answered`, `Completed FORM`, `Form submitted` / `Could not submit the form`.
- **Config:** `app/config/settings.toml` holds defaults, env wins (`llm_url`, `llm_model`, `[call_settings]
  default_language = "de"`, `[cache_settings] call_session_ttl_seconds = 3600`, `[llm_settings]`:
  `timeout_seconds = 120` because a cold GPU took 30 s+, `temperature = 0.0`, `max_tokens = 256` because an uncapped
  repetition loop ran to the context end). `.env` feeds compose and the app; `.env.stdin` has localhost values for
  the prompt loop and a local server; `.env.test` for `cargo test`. `APP_TIMEZONE` → container `TZ` → the
  assistant's "today". `FORM_SUBMIT_URL`: where completed forms go (empty = nothing sent; a host service from the
  container is `http://host.docker.internal:<port>/…`).
- Any change to `app/config/*.toml` or `client/index.html` needs an image rebuild (`./update-app.sh`).
- Cargo on Windows: a running `app.exe` blocks rebuilds; scratch copies need their own `CARGO_TARGET_DIR`.

---

## 2. Everything a caller can do

A call is always in one **flow**: the **main menu** (`CallState::Idle`) or a **form** (`CallState::FormInProgress`),
like menus in a game. Each flow has its own machines and intent menu, so the same words can mean different things in
each. Replies are quoted in English; every sentence code says exists in de/en/ru/uk in
`app/config/llm_vocabulary.toml`.

### 2.1 Anywhere in the call

| Caller does | Result |
|---|---|
| **Speaks another language** (de, en, ru, uk) in an utterance of **3+ words** | The call switches to that language; every later reply is in it. Shorter utterances ("Ja", "13. Juni 1991") keep the call's language (a bare German date was detected as English, "голова болит" as Ukrainian). A new call starts in German. The request's `language` field is only logged |
| Says something that makes a model call fail | Fixed apology, state put back as before the turn, still HTTP 200. Main menu: "Sorry, an error happened on our side, please try again. I can tell you…"; form: "…please say that again." The form keeps every value |
| Talks for long | Only the last 5 turns (10 messages) are kept as history for the models |

### 2.2 Main menu

| Caller says (examples) | Intent | Result |
|---|---|---|
| "What's the weather in Berlin?" | `get_information[get_current_weather_in_berlin]` | Live open-meteo data (5 s timeout) put into words by code ("Current weather in Berlin: overcast, 20 degrees Celsius, wind of 6 kilometres per hour from the west-northwest."), the formulator says it. On failure: an apology |
| "What is the euro to hryvnia rate?" | `get_information[get_current_uah_per_eur]` | Live NBU rate, same way |
| "I want to book a doctor's appointment" / **describes symptoms** ("I've had a headache for three days") | `start_form[doctor_appointment]` | Form starts. Code says "Sure, let's arrange a doctor's appointment." + the first step's question. No formulator |
| **Gives details before or with the booking request** ("This is Anna Schmidt, born 13 June 1991, I'd like to come tomorrow at 3 pm") | (any) + hints | The context extractor stores **hints** (`caller_full_name`, `patient_full_name`, `date_of_birth_iso_8601`, `appointment_spoken_date`, `appointment_spoken_time`, `appointment_reason`), kept for the call; a newer value replaces an older one. When the form starts they **prefill** it and each prefilled value is **read back for a yes** instead of asked for. A hint the field cannot read or a validator refuses (birth date in the future) is silently left out and asked for. `caller_full_name` fills nothing |
| "What did I book?" / "Which name did you write down?" (after a completed form) | `get_information[last_filled_out_form_information]` | Summary of the last completed form with its values. With none: says no form was filled out yet |
| **Books a second appointment** in the same call | `start_form[…]` | The completed form's name, birth date, day and time went back into the hints, so the new form starts **prefilled**, each value waiting for a yes. `reason` is asked again, unless the hints hold one (said before the first form, or taken from the history by the extractor) |
| "Which date is next Saturday?" | `get_information[calendar_help]` | Polite refusal: cannot help with dates |
| "Can you repeat that?" | `repeat` | The last reply again (formulator, from history) |
| "Goodbye" / "No, that's all" | `end_call` | Short goodbye, `action: end_call` (the phone side hangs up) |
| "Let me talk to a person" | `transfer_to_human` | `action: transfer_to_human` (only the action is set; transferring is the phone side's job) |
| **Only says hello** ("Hallo", "Guten Tag", "Hi, how are you?") | `greeting` | Says hello back and lists what `unsupported` lists, without an apology. A greeting that comes with a request is the request ("Hallo, ich möchte einen Arzttermin vereinbaren" starts the form) |
| Small talk, **anything similar but not supported** (another city, currency, a restaurant booking) | `unsupported` | Says so and lists every intent that has an `offer` in the vocabulary: "the euro to hryvnia exchange rate, the current weather in Berlin, booking a doctor's appointment" |

### 2.3 Inside the doctor-appointment form

Fields in order: `patient_name` (string) → `date_of_birth` (date, validated: not in the future) → `reason` (string)
→ `appointment_date` (spoken_date, **kept as said**, e.g. "next saturday") → `appointment_time` (spoken_time, kept
as said, e.g. "in the morning"). Each field is a step `queued` → `awaiting_confirmation` → `completed`; the **current
field** is the first not completed. Every reply that leaves the form open **ends with the current step's question**
(except after `repeat` and `transfer_to_human`): the field's `ask` ("What is the patient's date of birth?") or its
`confirm` with the value as spoken ("I have the patient's date of birth as June 13, 1991. Is that right?"; dates in
words, a text without the caller's final full stop). A field that got its value earlier is not asked for, only
confirmed.

| Caller says (examples) | Intent | Result |
|---|---|---|
| **Gives the asked value** ("Hans Müller") | `provide_form_field_value` | Recorded, then read back for a yes |
| **Gives several values at once** ("Hans Müller, born 13 June 1991, with back pain") | `provide_form_field_value` | **Every value is recorded**; each waits for its own yes, asked when the form reaches that field. Day + time in one answer ("tomorrow at 3 pm", "Morgen um 15 Uhr") fill both fields |
| **Says yes** to a read-back ("Yes", "Genau") | `confirm_yes` | Field confirmed. "Thank you." + next step. Only a clear agreement counts; a word that neither agrees, denies nor gives a value is `unsupported` |
| **Confirms the last field** | `confirm_yes` | "Thank you, that is everything. I have passed on your appointment request. Is there anything else I can do for you?" The form is **POSTed to `FORM_SUBMIT_URL`**, kept as `last_filled_out_form`, its values go back into the hints, and the call is back in the main menu |
| **Says no** ("No", "That's wrong") | `confirm_no` | Value dropped, rejection counter +1. "Sorry about that." + asked again. **From the 3rd no on one field:** "I'm sorry, I got that wrong again. If you prefer, just ask me for a human colleague." |
| **Corrects the read-back** ("No, it's 1992", "No, Hans Meier") | `correct_form_field_value` | The new value replaces it and is read back. A value changed in part is written whole ("no, the tenth" → full date) |
| **Agrees but changes it** ("Yes, but it's 1992") | `confirm_yes` → treated as a correction | The new value replaces the read-back one and waits for a yes. Other values in that answer are not taken |
| **Corrects an already confirmed field** ("Wait, the name is wrong, it's Hans Meier") | `correct_form_field_value` | That field reopens with the new value, read back. Plain answers never change a confirmed field |
| **Moves on**: answers the next question instead of yes (name read back, caller says "born 13 June 1991") | `provide_form_field_value` | The read-back value counts as **confirmed**; the new value is recorded and read back |
| **Points to a value** ("the same as before") | `refer_to_context_for_form_field_value` | Value taken from the conversation, read back |
| **Asks about the form** ("What name did you record?", "Is the appointment booked?", "Ist der Termin damit schon bestätigt?") | `get_information[form_information]` | The formulator answers from `<form_state>` in one sentence (code keeps the first; it never says booked/confirmed/done), then the step's question again. **A question is never a yes**: when the matcher says `confirm_yes`, the agreement checker (shown only the utterance) looks for a question, and the value stays unconfirmed |
| "Can you repeat that?" | `repeat` | Exactly the last reply again, word for word, no step after it |
| **Cancels** ("Forget it, cancel the booking") | `cancel_form` | Form dropped, nothing kept. "All right, I have cancelled the appointment request and kept nothing of it. Is there anything else I can do for you?" Back to the main menu |
| **Ends the call** mid-form ("Goodbye") | `end_call` | Form dropped, "Thank you for calling. Goodbye.", `action: end_call` |
| **Asks for a human** | `transfer_to_human` | Form **kept**, "I am transferring you to a human colleague now.", `action: transfer_to_human` |
| **Asks for something else** (weather, rate, calendar, another booking) | `unsupported` | "Sorry, I can't help with that right now." + the step's question. Main-menu features are not reachable from a form; finish or cancel first |
| **Unreadable value / no value** | (`provide` with nothing) | "Sorry, I didn't catch that." + the step's question |
| **Invalid value** (birth date in the future) | (any filling intent) | "A date of birth cannot be in the future." Not recorded; a read-back value it replaced is dropped; asked again |
| **Relative day/time** ("übermorgen", "next Saturday morning", "halb elf") | — | Stored as said: in German for a German caller, in English otherwise (ru/uk too). The receiver interprets it against `completed_at`. No date is ever computed |

A yes or no when nothing waits for confirmation changes nothing and the pending question is asked again. (A
yes-or-no field with no value would take the yes/no as its value; the doctor form has none.)

### 2.4 Not supported (yet)
No final "is everything correct?" over the whole form; no "end the call anyway?" inside a form; no `required` flag,
skip or "I don't know"; no `who_are_you` intent, no `greeting` inside a form; a question cannot deliberately end a call;
`end_call`/`transfer_to_human` only set `action` (two TODOs in `main_menu_intent_context_handler`); weather only for
Berlin, rate only EUR→UAH; one form type.

---

## 3. One turn, end to end

```
POST /assistant/handle-request   x-call-id: <id>   {"request_text": "...", "language": "en-US"?}
├─ call_session_middleware (route layer on this route only)
│    missing/empty x-call-id → 400 {"error": "x-call-id header is required"}
│    call_locks.lock(call_id): per-call tokio Mutex held until the session is saved (in-process only)
│    CallSession::from_or_new: Redis key call_id.<id> (CallData as MessagePack); an undecodable value = new call
│    CallSessionLoadedEvent → InitialContextHandler puts current_time ("Tuesday, 2026-10-06 09:25 +03:00")
│    after the handler: session.save(ttl 3600 s) on every request → sliding TTL
└─ handle_assistant_request (routes/api.rs): empty request_text → 400 → CallerSpokeEvent
     CallerSpokeHandler       1. lingua sets the call's language (3+ words of 2+ letters; numbers don't count)
                              2. the flow's CONTEXT EXTRACTOR: main menu → HintMap; form → ExtractedFormValues
     ContextExtractedHandler  main menu: merge hints, add <recently_completed_<form>_form> to the context
                              the flow's INTENT MATCHER (+ confidence from token logprobs)
                              form + confirm_yes → AGREEMENT CHECKER (utterance alone) → question? → form_information
                                (confidence then null, reasoning still the matcher's)
     IntentMatchedHandler     the flow's intent handler changes the state and returns a Reply:
                                Said(text)          code worded it: every form turn but questions; starting a form
                                Formulated{then}    RESPONSE FORMULATOR words it from response_context, code adds
                                                    `then` (the step's question): main menu, questions about a form
                              → save_last_exchange
                              completed form → completion callback (values → hints) → last_filled_out_form
                                → FormCompletedEvent → FormSubmitter
     any machine fails → session.fail_turn(utterance): state_before_turn restored, action continue, fixed apology
→ 200 {answer, selected_function, confidence, language_detected, action, reasoning, response_context, hint_map, form}
```

Model calls per turn: main menu 3; form 2, +1 tiny check (~90 ms on GPU) when the matcher said yes, +1 formulator for
a question. Handlers are async, built once at startup; per-call data travels in the event (`Arc<RwLock<CallSession>>`,
`Arc<AppState>`), and a handler releases the session lock before dispatching the next event. The prompt loop
dispatches the same two events per typed line (no lock).

### HTTP API

`POST /assistant/handle-request` (needs `x-call-id`), `GET /mimic-client`, `GET /version`, `GET /test-session`
(panics: expects a `UserSession` nothing adds; left alone). Bad requests → 400 `{"error": …}`. **A failed turn is a
200 with the apology** (`ApiError::Inference` is never produced).

| Key | From |
|---|---|
| `answer` | `data.last_spoken_response` |
| `selected_function`, `reasoning`, `confidence` | `call_turn_outcome` from the flow's intent handler; `null` when the turn failed before an intent |
| `language_detected` | the call's language, ISO 639-1 |
| `action` | `continue`, `end_call`, `transfer_to_human` |
| `response_context` | what the formulator was told; `null` when code worded the reply |
| `hint_map` | all hints (`null` = not given); `last_filled_out_form` is a whole `Form` or `null` |
| `form` | the form in progress after the turn; `null` in the main menu, so also on the turn that completes/cancels it |

Completed-form POST (spawned task, 10 s timeout, no retry; `call_id` lets the receiver deduplicate):
`{"call_id": "…", "completed_at": "2026-10-06T09:51:02.55+03:00", "form": {"kind": "doctor_appointment", "fields": [ … ]}}`.
Values serialize externally tagged, e.g. `{"spoken_date": "next saturday"}`.

### Mimic client (`client/index.html`, `include_str!`d into the binary)
Browser speech recognition + `speechSynthesis` (needs a secure context: localhost or https). One call id per page
load (reload = new call). Hangs up on `end_call` / `transfer_to_human` after the answer is read out. Shows every
response key under each answer: badges, confidence meter, `reasoning`, `response_context`, hints, the form (current
field marked) and `last_filled_out_form`. Newest line on top.

---

## 4. Code map

| Path | What |
|---|---|
| `app/config/llm_vocabulary.toml` | **Every text** (see §4.2). No text lives in Rust |
| `app/src/vocabulary.rs` | `vocabulary()` (LazyLock global), `Phrase`, `Localized` (de/en/ru/uk), `fill_in`, `Vocabulary::check()` |
| `app/src/domain/flow.rs` | Top: `FlowContext`, `IntentMatched`, `Reply`, `SpokenResponse`, `FormulatedResponse`. `main_menu_flow`: `HintMap` (`merge`, `prefill`, `form_values`, `last_filled_out_form_information`), its machines, `main_menu_intent_context_handler`, `start_form`. `form_flow`: its machines, `ExtractedFormValues` (`fit_schema`), `ExtractedFormIntent` (`is_agreement`, `as_question`), `CheckedAgreement`, `FormulatedAnswer` (`into_spoken_answer`), `form_intent_context_handler`, `validate_and_fill`, `fill`/`confirm`/`reject`, `next_step`, `given_values`, `holds`, `replaces_read_back`. Most behaviour changes happen here |
| `app/src/domain/form.rs` | `Form` (`current_field`, `fill_field`, `confirm_current`, `reject_current`, `answerable_fields`, `values_summary`, `context_value`), `FormField`, `StepState`, `FormFieldKind` (`parse`, `is_like`), `FormFieldValue::spoken`, `FormSupported::build()` (schemas), `::validate()` (validators), `::on_completed()` (callbacks), `ValidationError` |
| `app/src/domain/machine.rs` | `Machine { role, rules }`, `query`, `query_utterance_alone`, XML prompt rendering, the JSON schema, `ValueSchema` / `OutputFormat` / `AllowedValue` / `Described` |
| `app/src/domain/call.rs` | `CallerIntent` (labels, `from_label`), `CallAction`, `GetInformationSupported`, `FormSupported`, `flat_enum!` |
| `app/src/domain/information.rs` | `GetInformationSupported::fetch`: open-meteo weather, NBU EUR rate, worded with `[facts]` |
| `app/src/domain/call_session.rs` | `CallSession { call_id, call_turn_context, call_turn_outcome, data }`, `CallData` (`language`, `state`, `call_memory` {`conversation`, `hint_map`}, `last_spoken_response`), `context()`, `save_last_exchange`, `fail_turn`, `CallLocks` |
| `app/src/event/*.rs` | One event + handler per file, in turn order: `call_session_loaded`, `caller_spoke`, `context_extracted`, `intent_matched`, `form_completed` (`FormSubmitter`); `events.rs` registers them, `event_bus.rs` dispatches |
| `app/src/vllm.rs` | `VllmClient::query` (sends model, temperature, max_tokens, `logprobs: true`, `[system, user]`, strict `json_schema`; bearer only with `LLM_API_KEY`), `Answer::probability_of(key)`, `VllmError` (`Timeout`, `Transport`, `ContextWindowFull`, `BadResponse`, `Malformed` = cut at `max_tokens`); every error = a failed turn |
| `app/src/classifier.rs` | lingua `detect_language` (en, de, ru, uk), `MIN_WORDS = 3` |
| `app/src/routes/` | `api.rs` (routes, handler), `call_session_middleware.rs` |
| `app/src/app.rs`, `settings.rs`, `error.rs` | `AppState` (db, cache, session, llm, http_client, settings, event_dispatcher, call_locks); `AppSettings`, `is_prompt_loop_mode`; `ApiError` |
| `client/index.html` | Mimic client |
| `evals/` | DeepEval suites, §7 |
| `docker/vllm/` | vLLM image wrapper. `docker/classifier/` is an abandoned GLiNER2 experiment (machine #1 without an LLM), not in compose, not called |
| `migrations/` | One test table; `sqlx::migrate!` is commented out in `main.rs` |

### 4.1 Machines and prompts (`machine.rs`)

| Flow | Machine | Output |
|---|---|---|
| main menu | `main_menu_context_extractor` | `HintMap` (6 hints, each string or null; spoken date/time in the caller's words) |
| main menu | `main_menu_intent_matcher` | `machine_reasoning`, `caller_intent` + confidence |
| main menu | `main_menu_response_formulator` | `spoken_response` |
| form | `form_context_extractor(language)` | `ExtractedFormValues`: one key per answerable field, string or null. Dates as `YYYY-MM-DD`; spoken day/time lowercase, German for a German caller (German examples in its rule), English otherwise; null for all keys on yes/no/questions/requests |
| form | `form_intent_matcher` | `machine_reasoning`, `caller_intent` + confidence |
| form | `form_agreement_checker` | `is_question` (shown the utterance only) |
| form | `form_response_formulator(language)` | `spoken_response`; first rule names the reply language |

Intent menus. **Main menu:** `unsupported`, `greeting`, `get_information[calendar_help]`,
`get_information[get_current_uah_per_eur]`, `get_information[get_current_weather_in_berlin]`,
`get_information[last_filled_out_form_information]`, `start_form[doctor_appointment]`, `repeat`, `end_call`,
`transfer_to_human`. **Form** (in this order):
`unsupported`, `provide_form_field_value`, `correct_form_field_value`, `refer_to_context_for_form_field_value`,
`confirm_yes`, `confirm_no`, `cancel_form`, `get_information[form_information]`, `repeat`, `end_call`,
`transfer_to_human`. Each flow lists its intents in `allowed_values()` of its `Intended…Action` newtype.

System message (user message is `<utterance>…</utterance>`):
```xml
<role>…</role>
<rules><rule>Set `field` to: …[. Use only a value listed in <allowed_values><field>]</rule>…own rules…</rules>
<allowed_values><caller_intent><allowed_value><value>confirm_yes</value><description>…</description></allowed_value>…
<json_output_format>{ "machine_reasoning": "string", "caller_intent": "string" }</json_output_format>
<conversation_history>…last 5 turns, omitted when empty…</conversation_history>
<context><current_time/><form_state>{json}</form_state><language/><response_context/></context>
```
- `<context>` is `CallSession::context()`, sorted by key so `response_context` (formulators only) comes **last**,
  next to the utterance; the 7B model follows it poorly anywhere else. `<form_state>` is in every form-turn prompt.
  `<recently_completed_<form>_form>` (the `values_summary()` of `last_filled_out_form`) is added in the main menu after
  hints are merged, so the matcher and formulator see it and the extractor does not (with it, the extractor copied
  the old form's values into hints on unrelated turns).
- The agreement checker (`query_utterance_alone`) gets role, rules and output format only; no history, no context.
  Its texts must not name a tag it lacks (a test checks).
- Role, rules and allowed-value descriptions are verbatim (may name tags); everything else is escaped for `<>&`, so a
  tag written inside `response_context` arrives as `&lt;…&gt;`.
- `<json_output_format>` comes from schemars in struct order (`serde_json/preserve_order`, workspace-wide). An
  `Option` is shown as `"string"`, never `["string","null"]` (the model copied that as an array).
- The JSON schema (`json_schema()`): only the output's fields, no empty strings, strict fields limited to allowed
  values, `null` only where `ValueSchema::accepts_null` (only `HintMap`; without it the model wrote "null"/"string").
  `OutputFormat::fit_schema` adapts it per call (only `ExtractedFormValues`: keys = `Form::answerable_fields`).
  `OutputFormat::read_answer` reads extras such as confidence.

### 4.2 Vocabulary (`config/llm_vocabulary.toml`)

| Section | What | Found by |
|---|---|---|
| `[phrases.<name>]` | Sentences code says: `thanks`, `sorry`, `offer_human`, `not_understood`, `cannot_help_in_form`, `goodbye`, `transfer`, `turn_failed_in_main_menu`, `turn_failed_in_form` | `Phrase::Thanks.say(language)` |
| `[validation.<name>]` | Validator messages | `ValidationError::….say(language)` |
| `[dates]` | `format` (`{day}`, `{month}`, `{year}`) and 12 `months` per language | `FormFieldValue::spoken` |
| `[forms.<form>]` | `started`, `completed`, `cancelled` | `form.kind.vocabulary()` |
| `[forms.<form>.fields.<field>]` | `description` (what machines read), `ask`, `confirm` with `{value}` | `.field(name)`, `next_step(form, language)` |
| `[intents.<label>]` | `description` for matchers; `offer` = named in the main-menu refusal and the reply to a greeting | `intent.description()` |
| `[machines.<name>]` | `role`, `rules` (form extractor also `examples` per language) | `Machine::form_intent_matcher()` etc. |
| `[output_values]`, `[prompt]` | Descriptions of answer keys, the per-key rule | `ValueSchema`, `render_system_prompt` |
| `[instructions]`, `[facts]` | What a formulator is told; weather/rate sentences, WMO sky codes, 16 wind directions | formulator, `fetch` |

- Caller texts need all four languages (`Localized` has no "other" arm: a new detector language does not compile
  until the vocabulary has it); machine texts are one English string.
- **A missing text stops the app at startup** (`Vocabulary::check()` in `main` and a test; unknown keys fail via
  `deny_unknown_fields`). Keys are snake_case variants, intent labels (`"get_information[calendar_help]"`) or field names.
- Still in code: intent labels, field names, prompt tags, log lines, panic/400 messages.

### 4.3 Hints (`HintMap`)
Kept in `call_memory.hint_map` for the whole call. Every main-menu turn merges (new value replaces, missing keeps).
Form turns never touch hints except the completing one. `last_filled_out_form: Option<Form>` is not a hint
(`#[schemars(skip)]`, not merged), set by `IntentMatchedHandler` after the completing reply. Prefill on form start
(`HintMap::prefill`, runs validators, refused hints dropped silently):
`patient_full_name`→`patient_name`, `date_of_birth_iso_8601`→`date_of_birth`, `appointment_spoken_date`→
`appointment_date`, `appointment_spoken_time`→`appointment_time`, `appointment_reason`→`reason` (2026-10-07; a
symptom, complaint or examination, not the wish for an appointment). `doctor_appointment_completed` writes the first
four back, not the reason. Hints stay in the caller's language; form values are written in English (German for German callers' days/times).

### 4.4 Forms, validators, completion
- Field kinds and `parse`: `string` (non-empty), `unsigned_integer`, `integer`, `float` (comma = decimal point),
  `bool` (true/yes, false/no), `date` (ISO), `spoken_date`, `spoken_time` (non-empty text as said).
- **No two neighbouring fields may take a like kind of value** (`is_like`: both dates, both numbers, or equal);
  `spoken_date` and `spoken_time` are not alike. The builder asserts it; test `every_form_builds`.
- **Validators:** `async fn(&AppState, &FormFieldValue) -> Result<(), ValidationError>`, chosen by
  `match (form, field name)` in `FormSupported::validate`. Not stored in `FormField` (the form goes to Redis, prompts
  and the POST). They run in `validate_and_fill` and `HintMap::prefill`, not on a yes/no taken as a bool value.
  `AppState` reaches them through the events. Only one exists: `date_of_birth` not after today (on `current_time`'s clock).
- **Completion:** last field confirmed → state `Idle`; after the reply: log `Completed FORM`, `on_completed`
  callback (hints), `last_filled_out_form`, `FormCompletedEvent` → `FormSubmitter` POST. With no URL: a warning, nothing sent.

---

## 5. Rules decided in code, not by the model (and why)

1. **Extracted values only count when the intent fills a field** (`IntentMatched::Form(intent, values)`): the
   extractor writes held and historical values on many yes/no/question turns.
2. **A value the field already holds is not a value the caller gave** (`given_values`, `holds`): otherwise a plain
   "нет" taken for a correction re-recorded the old name.
3. **A correction without a value is a rejection.**
4. **Values for later fields alone confirm the read-back field** (the caller moved on). Only for provide/refer,
   never a correction, not when the answer also gives the current field. Decided once per answer before filling.
5. **An agreement with another value is a correction** (`replaces_read_back`), only when sure: the kind can read it,
   and for text the new words are in the utterance, for a date/number the value differs. Never for a bool field.
   Only the current field's value is taken.
6. **Another value in place of the read-back one says that one is wrong**, even if unreadable or refused: dropped,
   asked again.
7. **A refused value is not recorded.**
8. **A completed form is sent only after its reply is worded**, so a failed turn sends nothing.
9. **The extractor's keys are the answerable fields** (`answerable_fields`): every field except a later one with a
   kind like the current one (unless the current takes text). While `date_of_birth` is the step, `appointment_date`
   is no key: "Nein, am vierzehnten" was filed under the appointment in 3 of 3 replays with it.
10. **Answers never change a confirmed value**; only a correction does (the matcher chose correction for "Moment, der
    Name ist falsch, …" in 11 of 11 logged turns).
11. **Code words every form reply** from the vocabulary; the formulator only answers questions about the form (first
    sentence kept) and words main-menu replies. Left to the formulator, replies read back every value, skipped
    confirmations, repeated sentences or claimed early booking.
12. **A question is never an agreement** (agreement checker on what the matcher took for `confirm_yes`).

---

## 6. Evidence: why it is built this way

**Method.** One live run says little: vLLM is not fully deterministic at temperature 0 when two answers are close,
and one changed reply changes the history of every later turn. **Compare wordings by replaying every logged `Machine
prompt` of one kind with the change applied, and count** (German and English, Russian too). A replayed matcher prompt
reproduces the logged answer (1,832 of 1,832). History: one big prompt (2026-09-17), two machines per turn
(2026-09-26), two flows with three machines each (2026-10-05), code-worded form replies and the agreement checker
(2026-10-06).

- **Code words the replies (2026-10-06).** 580 logged formulator prompts replayed, flagged = fails a reply check,
  says an unasked value, repeats a sentence, says a field description verbatim, invents weather, speaks an intent
  label, or asks "is that right?" after completion: formulator as it was 167 (106 de / 61 en); best prompt shape
  found 71; **code words form steps 2**; Qwen3-8B-AWQ 218 old / 59 best / 2 with code. Suites before → after: goldens
  outside known gaps 82% → 94% de, 65% → 92% en; Repeated Sentences 7/18 → 18/18; form turn median 1133 → 759 ms.
  Tried and not kept: a `pattern` requiring a final "?" (junk to `max_tokens`), `maxLength` (cuts sentences), "thank
  them in one or two words" (drops the step), an English draft the model translates, history as chat messages.
- **Answers to questions about the form keep the first sentence.** A JSON-schema `pattern` forbidding "?" made the
  model write an Arabic question mark, run to `max_tokens`, or vLLM returned 500 (`grammar rejected tokens`).
  Without it 16/49 answers held a question and 25 had more than one sentence, but the first sentence fit 49/49.
  Naming the reply language in the formulator's rule ("Reply in German, which is the language the caller speaks")
  stopped German questions being answered in English (6 → 1 of 57). The rule "never say it is booked, confirmed or
  done" cut such answers 22 → 13 of 1,162.
- **A question is never a yes (2026-10-06).** "Ist der Termin damit schon bestätigt?" was a yes in 19/20 and
  confirmed the read-back value. Over 1,344 unseen questions at 56 form points: 535 taken for yes. Rules in the
  matcher's prompt left 119–139 and moved unrelated answers; an `is_question` key moved 49 of 1,196 other answers; a
  separate check shown the call missed 9 and took 280 ms; **shown the utterance alone: 0 missed, 0 of 2,003
  agreements taken for a question, 86 ms median.** Live after: the question 19 → 0 yes; 140 questions matched as a
  question 97 → 116; questions that change the form or call 11 → 1. Backfired: `machine_reasoning` before its answer
  (15 agreements lost), "true when the caller only asks something…" (354).
- **Several values per answer.** One key per field instead of one field/value pair: "Morgen um 15 Uhr", "Tomorrow at
  3 pm" etc. filled both fields 13/13; a long first sentence gave name, reason, day and time (call 16 → 8 turns).
  The held field name trick took day+time answers 36 → 45 of 48. The extractor's examples decide the language of a
  spoken day: English examples turned "Übermorgen" into "next monday"; mixed examples leaked across languages;
  German examples for German callers is what's built ("übermorgen" 5 of 6).
- **Relative dates kept as said** (2026-09-26): asked for `YYYY-MM-DD` the model got 1 of 8 right ("next Saturday" →
  a Wednesday), 0 of 6 even with a 15-day calendar; copying the caller's words worked 6 of 8.
- **A greeting has its own intent (2026-10-07).** As `unsupported` it was told "The caller asked for something that
  is not supported", and 23 of 37 replies to a greeting (de/en/ru/uk) apologised ("Hallo! Tut mir leid, aber das, was
  Sie suchen, wird nicht unterstützt."); told "The caller said hello. Say hello back and list…" 0 did (1 dropped an
  offer). With `greeting` after `unsupported` in the menu, of 407 logged main-menu matcher moments 14 moved: 12
  greetings and "I don't speak German" to `greeting`, one other answer. New probes: 32 of 32 greetings (de/en/ru/uk)
  matched, 17 of 17 greetings with a request stayed the request. Last in the menu it took "Have a nice day" from
  `end_call`; "…A thank-you or a goodbye is not a greeting" in the description made "Danke" and "Дякую" greetings.
  The `unsupported` description still names a greeting: the form's matcher shares it and has no `greeting`.
- **`calendar_help` is not in the form menu:** "в следующую субботу" as an appointment date was matched as a
  calendar question (0.9998) and the form could not finish.
- **Field description wording matters:** "Preferred date of the appointment" made German replies ask for "den
  bevorzugten Termin für den Termin" (38 → 46 of 47 right with "Date of the appointment").
- **Language rule:** of 151 utterances, 9 mis-tagged when every utterance sets the language, 4 with the 3-word rule.
- **HintMap nullable keys:** junk values ("string", "null") from 7–13 per 26 calls to 0 in 42.
- **Matcher wording for denials:** describing `confirm_no` as "A denial that gives no other value…" cut plain
  denials taken for a correction 30 → 12 of 65; every "yes, but X" / "no, X" is a correction (59/59).

### What the 7B model does badly
- Any date arithmetic, with or without a calendar.
- Follows `response_context` poorly unless it is last; repeats its previous question.
- Copies type hints literally (`["string","null"]` → arrays), writes "null" unless null is allowed.
- Takes a missing year from other dates in the history; copies examples from rules into output.
- Drifts into Chinese inside Russian replies. Confidence does not point at its mistakes (wrong intents at 0.9989, 0.9998).
- Invents facts from raw data (weather code 3 "sonnig", wind from 287° "aus Südwesten", labels spoken as words).
  Hence facts are put into sentences by code.
- Words a step its own way when told *what to do* rather than *what to say*.
- Takes a question about what the form waits for as agreement (40% with the form in its prompt, 0% with the
  utterance alone). A rule fits only the wording it was written for and moves unrelated answers.
- Does not respect a JSON-schema `pattern` that forbids what it wants to write.

---

## 7. Tests and evals

```bash
cargo check -p app --all-targets                            # 31 warnings, see gap 19
cargo test -p app -- domain:: vocabulary:: classifier::     # 23 unit tests, no services needed
cargo test -p app                                           # + tests.rs (config, Postgres, Redis; .env.test)
```

**Evals** (`evals/`): DeepEval suites that talk to the **running** app over HTTP like the phone side, check every
turn part by part, and write a report. German first, English second; no goldens in other languages.

```bash
./run-deepeval-tests.sh                       # every suite (~5 min on GPU, ~18 min on the Mac)
./run-deepeval-tests.sh -r 3                  # every golden 3x: pass rates instead of one sample
./run-deepeval-tests.sh test_form.py -k de    # one suite + any pytest args (-n = parallel, ruins timings)
./run-deepeval-tests.sh --report              # report of the latest run again (recomputes checks)
```
The script creates `evals/.venv` (Python 3.11) on first use, reinstalls `requirements.txt` when it changes, checks
the app answers, points `EVAL_LLM_URL` at Ollama when it runs, and passes its arguments to `evals/run.py`.
`run.py` calls `deepeval test run`, writes `evals/results/report_<time>.md` next to `test_run_<time>.json`, prints
the summary, exits 1 if a golden failed. DeepEval's own "pass rate" counts Turn Latency; read the report instead.

| Variable | Default | |
|---|---|---|
| `EVAL_APP_URL` | `http://localhost:8080` | the app |
| `EVAL_REDIS_URL` | `redis://localhost:6379` | to copy a call's session |
| `EVAL_APP_LOG_DIR` | `../logs` | per-machine timings |
| `EVAL_TURN_BUDGET_MS` | `2000` | Turn Latency limit |
| `EVAL_JUDGE_MODEL` | none | judge for LLM-judged metrics |
| `EVAL_LLM_URL` | `http://localhost:8000/v1` | the model server (judge `local`, and the model name in the report; on the Mac `http://localhost:11434/v1`) |

| Suite | Goldens | What |
|---|---|---|
| `test_main_menu.py` | 28 de, 17 en | weather, rate, starting the form (also from symptoms), calendar refusal, repeat, end call, transfer, a greeting (alone and with a request; goldens not run yet), near-miss requests, booking question with no form yet |
| `test_hints.py` | 8 de, 6 en | hints before the form: kept over turns, prefilled for confirmation, refused by a validator |
| `test_form.py` | 69 de, 30 en | one answer at one point of the form: value, yes, no, "no, it is…", "yes, but…", next field's value, several values, questions (incl. "Ist der Termin damit schon bestätigt?" also without "?"), outside request, repeat, cancel, transfer, end call, validator, completion, main menu after completion |
| `test_conversations.py` | 10 de, 3 en | whole calls: full sentences, short answers, hints, corrections, three rejections → human, interruptions, questions where a yes was due, pointing to a value, language change |
| `test_api.py` | 10 | bad requests, response keys, session sliding expiry, one turn at a time, a failing turn, completed-form handling |

- `FormStates` in `suite.py` walks one call per language through the form and `COPY`s its Redis session after every
  step, so each form golden starts from exactly that state. Consequence: goldens of one step share a walk and
  pass/fail together; two runs do not rank two wordings.
- A golden showing a known gap carries `known_gap="… (gap N)"` (N = §9 numbering); it is marked flaky and does not
  fail the run.
- Goldens name code's sentences by vocabulary key (`phrase`, `refusal`, `form_says`, `ask`, `confirm` in `suite.py`),
  so rewording the vocabulary needs no golden change. Adding one:
  `golden("birth-no-other-year-de", "Nein, 1992.", state="dob_awaiting", lang="de", capability="correct a value",
  intent=CORRECT, form={"date_of_birth": ["awaiting_confirmation", "1992-06-13"]})`. Values: `None`, exact text, or
  `has("a", ("b", "c"))`.
- **Checks** (`metrics.py`, deterministic, no model): Language Detection, Hint Extraction, Intent, Form State,
  Completed Form, Call Action, Response Context (only on formulated turns), Reply Language, Reply Speakable (no
  markup/snake_case/ISO date/emoji/foreign script), Reply Says, Reply Asks (≤1 question, the form's next step, via
  `ASKS_FOR` words; reworded `ask` sentences must keep one of them), No Early Booking, Turn Latency, Repeated
  Sentences (vocabulary sentences don't count), Facts. Reply Asks was right on 176 of 177 hand-read replies.
- **Report sections:** what to look at first; per-check pass rates by language; where a failing turn fails first
  (in turn order); per-capability results; where the time goes per machine (prompt/answer sizes); does confidence
  point at mistakes; known gaps; every failed golden with what was said and heard.
- **Judged metrics** (`EVAL_JUDGE_MODEL=gpt-4.1` + `OPENAI_API_KEY`, `claude-opus-5-5` + `ANTHROPIC_API_KEY`, or
  `local`): Follows Instruction (GEval), Faithfulness, Form Manners, Knowledge Retention. Off by default: the local 7B
  agreed with hand labels only 10/13 (GEval), 12/16 (PromptAlignment), 20/36 (DAG). Not yet run with a strong judge.
- Limits: step timings use the app's (Docker VM) clock, ~3% fast and occasionally set back (such turns are skipped);
  weather/rate goldens need network; form delivery to the receiver is not checked; calls stay in Redis until expiry;
  each failed turn logs ~130 kB of prompt.

**Baselines**
- vLLM AWQ, RTX 3080 Ti, 2026-10-06, `-r 2`: goldens outside known gaps **94% de / 92% en**; form turn median
  759 ms / p95 1201; main menu 828 / 1302 ms.
- Mac, Ollama `qwen2.5:7b-instruct`, 2026-10-07, `-r 1` (`report_20261007_003734.md`): **90% de / 89% en**; turn
  median ~3.1 s / p95 5.9 s, extractor + matcher ≈ 95% of a form turn. The three `failed-turn-*` goldens always fail
  here (they need vLLM's 400 for an over-long prompt). Compare Mac runs with Mac runs.

---

## 8. How to extend

- **Form field:** a line in `FormSupported::build()` + `[forms.<form>.fields.<name>]` with `description`, `ask`,
  `confirm` (`{value}`) in de/en/ru/uk. Mind `is_like` for neighbours. Hint mapping in `HintMap::form_values`,
  callback in `on_completed`. **Renaming a field:** also rename it in `validate` and `form_values` (nothing fails
  otherwise; the validator just stops running).
- **Validator:** async fn + an arm in `FormSupported::validate`; a new reason = `ValidationError` variant +
  `[validation.<name>]`. Several: `first(s, v).await?; second(s, v).await`.
- **Sentence code says:** `Phrase` variant + `[phrases.<name>]` in four languages.
- **Intent:** `CallerIntent` variant/label (`call.rs`), `[intents.<label>]` description (+ `offer`), the flow's
  `allowed_values()`, a match arm in the flow's intent handler.
- **Machine output field:** a newtype implementing `ValueSchema` with a key in `[output_values]`, inside a struct
  deriving `JsonSchema, Deserialize, Default` implementing `OutputFormat::iter_schemas` (keys = serde names). Keys
  that depend on the call: a map with `fit_schema`, like `ExtractedFormValues`.

---

## 9. Known gaps (numbers are cited by evals `known_gap`)

**Language**
1. The call's language follows every 3+ word utterance and the detector errs: "What is the euro to hryvnia exchange
   rate?" / "How many hryvnias do I get for one euro?" → German; ru/uk confused in 3–5 word sentences ("Так, усе
   правильно." → ru), and form replies then switch language. A call opening with <3 words stays German. Fix ideas:
   `with_minimum_relative_distance` on the lingua builder, or trust the language the phone side sends.
2. Chinese inside Russian formulator replies (main menu and question answers now; 8 of 143 before code worded form
   replies). A `pattern` on `spoken_response` fixed all 8 on replay, not applied:
   `^[ -~ -ɏЀ-ӿ‐-‧‰-›]+$`. (Caution: patterns can derail the
   model, see §6.)
3. ru/uk vocabulary written without a native speaker; avoids gendered verbs ("Имя пациента — {value}. Всё верно?").
4. Some Russian names come out transliterated in hints.

**Dialogue**
5. A question about the form is recognised ~8/10 (116/140). Left: "Are we done?" / "Do you need anything else?" →
   `end_call` (drops the form; 48 of 1,344); "Steht der Termin jetzt?" → `confirm_no` (3/720); "Wäre 15 Uhr
   möglich?" during a read-back counts as moving on (46/120); "Wie viele Fragen kommen noch?", "Warum fragen Sie
   das?" → `unsupported`, "Welche Angaben brauchen Sie noch von mir?" → answer without value; ru/uk yes-no questions
   without "?" read as statements (may count as yes); "Ja. Und jetzt?" / "Ja, stimmt, oder?" treated as questions;
   13/1,162 answers still claim a booking or promise e-mails. A question about the read-back value hears it twice;
   main-menu small talk sometimes misses the offer list. Next to a greeting: "Schönen Tag noch", "Ciao", "Danke
   schön" and "Wer sind Sie?" were taken for `greeting` on replay; "Servus" / "Moin" are answered with "du".
6. A value matched as `confirm_yes` while its field has no value is lost ("June 13th 1991" after a refused "no, it
   is 2092", 1 of 6).
7. Unclear words ("хм", "так", "ну", "hmm") taken for `confirm_yes` in 7–9 of 160. `repeat` in the main menu refers
   to `<conversation_history>`, which arrives escaped (the form flow quotes the reply instead).
8. "Andrew with a v" is recorded with the words in it.
9. On a question about the completed form the main-menu extractor fills hints from history (harmless now: the hints
   hold those values). `form_information` (form only) and `last_filled_out_form_information` (main menu only) are
   two intents.
10. Several values per answer: values for later fields said with a yes are lost; an answer the asked field cannot
    take may be recorded for a later field it fits ("Irgendwann im Sommer" as the time, 1 of 3); ru/uk days/times
    are written in English (not measured); a held day/time re-extracted in another language counts as a new value
    for a read-back field. Not built: final confirmation, "end anyway?", `required`, skip, a `who_are_you` intent.
11. `end_call` and `transfer_to_human` only set `action`.
12. The menu depends on the flow, not the state inside it: `confirm_*` is offered with nothing pending.

**Correctness and safety**
13. PII in logs: full prompts at `info`, the completed form in `Completed FORM` and `Form was submitted!` (logged
    before the POST).
14. History is capped at 5 turns, but a very long utterance still ends in `ContextWindowFull` on vLLM (a failed turn).
15. `x-call-id` is trusted: ids must be unguessable or authenticated by the telephony side.
16. The call lock is per process; several app instances would race (last write wins).
17. Form submission is not retried; a failed POST is only logged.
18. Validators have no test of their own (needs an `AppState` → Postgres + Redis). A form saved in Redis by an older
    build with a field the vocabulary no longer has panics on its question until the session expires.

**Leftovers**
19. 31 compiler warnings: cookie-session middleware and `session.rs`, `/test-session`, `db`/cache helpers, unread
    `AppState` fields, unused event-bus parts, `llm_system_prompt_file`, `session_ttl_days`, `VllmClient::model` /
    `query_json`, four `AllowedValue` variants, `call_id` fields of two events, unused imports.
20. Prompt-file remnants: `llm_system_prompt_file` in `settings.toml`/`settings.rs`, `LLM_SYSTEM_PROMPT_FILE` and the
    `./app/prompts` mount in compose, the `COPY` in the `Dockerfile` (see §1 build gotcha).
21. `docker/classifier/` is unused.
22. `sqlx::migrate!` is commented out in `main.rs`; `ApiError::Inference` is never produced.

---

## 10. Working agreements

- Keep it stupid simple; do only what the current step asks. The user commits; their edits between steps are
  deliberate.
- Match the style: a newtype + `ValueSchema` per machine-output field, `/* --- */` separators, few comments, no
  text in Rust (vocabulary only).
- Change prompts only with evidence: replay many logged prompts (German, English, Russian) and count; confirm with
  `evals/run.py -r 2`+.
- Research established practice before designing a dialogue mechanism. Earlier ideas still valid: record where a
  slot value came from, idempotent commits, hand off after N failed turns, order prompt layers static → volatile
  (helps prefix caching).
- When the tree doesn't compile for unrelated reasons, verify in a scratch copy with its own `CARGO_TARGET_DIR`.
