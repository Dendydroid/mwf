# Context — ai_assistant

**Updated:** 2026-10-06 · **Repo:** `H:\rust\ai_assistant` (branch `master`, last commit `a631322 before bench`; the
form completion callback, the two appointment fields and `evals/` are uncommitted) · **Stack:** Rust / axum 0.7 ·
Redis (call sessions) · Postgres (connected, not used by the assistant) · vLLM serving `Qwen/Qwen2.5-7B-Instruct-AWQ`
over the OpenAI-compatible API, on an RTX 3080 Ti (12 GB) that also drives the Windows desktop · lingua for the caller's
language
**Reference project:** `H:\rust\ai_autocalls_rust` · **Background:** `docs/call-assistant-handbook.html`

Earlier designs: two machines per turn, 2026-09-26 (`git show 3bb3987:context.md`), and the single prompt of 2026-09-17
(`git show fd90a3d:context.md`). `updates.md` is the log of the refactor of 2026-10-05 with what its runs measured. This
file says what the code is now.

---

## What it is

The backend of a phone assistant. Each caller utterance is one HTTP request, tied to a call by the `x-call-id` header,
or one line typed into the prompt loop.

A call is always in one **flow**, like in one menu of a game: the **main menu**, or a **form** once it is started
(`CallState::Idle` and `CallState::FormInProgress(form)`). Each flow has its own three LLM "machines", so they only know
what the caller can say in it:

1. **Context extractor** takes values out of the utterance.
2. **Intent matcher** picks one intent of the flow's menu.
3. **Response formulator** writes the sentence the caller hears.

Between the second and the third, Rust code changes the call's state and tells the formulator exactly what to say
next. **The model extracts, code decides.** Every turn is three model calls, about one second in a form on the local GPU.

Supported: current weather in Berlin, EUR→UAH rate, one form (doctor appointment) filled field by field with each
value confirmed and checked by validators, questions about the form, a refusal for calendar questions, repeat, end
call, transfer to a human, and a polite refusal listing what is supported. A completed form is POSTed to another
project.

---

## One turn, end to end

```
POST /assistant/handle-request   header x-call-id: <id>   body {"request_text": "...", "language": "en-US"?}
│
├─ call_session_middleware — a route layer on this route only (routes/api.rs::router)
│    missing/empty x-call-id → 400 {"error": "x-call-id header is required"}
│    state.call_locks.lock(call_id)             per-call lock, held until the session is saved
│    CallSession::from_or_new(call_id)          Redis key call_id.<id>, or a new call in the default language
│    dispatch CallSessionLoadedEvent → InitialContextHandler puts current_time into the turn's context
│    after the handler: session.save(ttl 3600 s) on every request → sliding TTL
│
└─ handle_assistant_request (routes/api.rs)
     empty request_text → 400
     dispatch CallerSpokeEvent
       CallerSpokeHandler        1. lingua detects the language of the utterance and sets the call's language
                                 2. the flow's context extractor
                                      main menu → HintMap
                                      form      → ExtractedFormValue (one field and its value)
       ContextExtractedHandler   main menu: merges the hints into the call's HintMap
                                 the flow's intent matcher → ExtractedMainMenuIntent | ExtractedFormIntent
       IntentMatchedHandler      the flow's intent context handler changes the state and writes response_context
                                 the flow's response formulator writes the reply → save_last_exchange
                                 a completed form → its completion callback writes values back into the HintMap
                                                  → FormCompletedEvent → FormSubmitter POSTs it
     a machine that fails → session.fail_turn(utterance): the state is put back, the caller hears a fixed apology
     → 200 {"answer", "selected_function", "confidence", "language_detected", "action", "reasoning",
            "response_context", "hint_map", "form"}
```

`cargo run -- --prompt-loop` does the same without HTTP: one call id per process, and for every typed line it loads
the session, dispatches the same two events, saves and prints `Response: …`.

---

## File map

| path | holds |
|---|---|
| `app/src/domain/flow.rs` | At the top what both flows share: `FlowContext`, `IntentMatched`, `Reasoning`, `SpokenResponse`, `FormulatedResponse`. Then `main_menu_flow` (`HintMap`, its machines, `main_menu_intent_context_handler`, `start_form`) and `form_flow` (its machines, `form_intent_context_handler`, `fill` / `confirm` / `reject`, `next_step`) |
| `app/src/domain/form.rs` | `Form`, `FormField`, `FormFieldKind` (+ `parse`, `is_like`), `FormFieldValue`, `StepState`, the form schemas in `FormSupported::build()`, the validators in `FormSupported::validate()`, `ValidationError`, the completion callbacks in `FormSupported::on_completed()` |
| `app/src/domain/machine.rs` | `Machine`, `query`, XML prompt rendering, the JSON schema sent to vLLM, `ValueSchema` / `OutputFormat` / `AllowedValue` / `Described` |
| `app/src/domain/call.rs` | `CallerIntent` (labels, `all`, `from_label`, the descriptions the matchers see), `CallAction`, `GetInformationSupported`, `FormSupported`, `flat_enum!` |
| `app/src/domain/information.rs` | `GetInformationSupported::fetch` (open-meteo weather, NBU EUR rate, fixed texts for the other two) |
| `app/src/domain/call_session.rs` | `CallSession`, `CallData`, `CallState`, `CallMemory` / `CallTurn` / `Transcript`, `CallTurnOutcome`, `context()`, `save_last_exchange`, `fail_turn`, `CallLocks` / `CallLock` |
| `app/src/event/call_session_loaded.rs` | `CallSessionLoadedEvent`, `InitialContextHandler` |
| `app/src/event/caller_spoke.rs` | `CallerSpokeEvent`, `CallerSpokeHandler` |
| `app/src/event/context_extracted.rs` | `ContextExtractedEvent`, `ContextExtractedHandler` |
| `app/src/event/intent_matched.rs` | `IntentMatchedEvent`, `IntentMatchedHandler` |
| `app/src/event/form_completed.rs` | `FormCompletedEvent`, `FormSubmitter` |
| `app/src/event/events.rs` | registration of all five, `events(&settings, &http, &llm)` |
| `app/src/event/event_bus.rs` | the dispatcher |
| `app/src/classifier.rs` | `detect_language` (lingua: English, German, Russian, Ukrainian) |
| `app/src/vllm.rs` | `VllmClient` (`query`, `query_json`), `Answer` (+ `probability_of`), `VllmError` |
| `app/src/routes/call_session_middleware.rs` | the middleware above |
| `app/src/routes/api.rs` | routes, `handle_assistant_request` |
| `app/src/app.rs` | `AppState`: `db`, `cache`, `session`, `llm`, `http_client`, `settings`, `event_dispatcher`, `call_locks` |
| `app/src/settings.rs` | `AppSettings`, the only `is_prompt_loop_mode` |
| `app/src/error.rs` | `ApiError` → `{"error": "..."}` |
| `client/index.html` | the mimic client, `include_str!`d into the binary (a change needs an image rebuild) |

An event file holds its event and its handler and nothing else. `event/mod.rs` lists them in the order a turn
dispatches them.

---

## vLLM client (`vllm.rs`)

- `query(system, user, name, schema) -> Result<Answer, VllmError>`. `Answer` is the model's raw JSON text plus the log
  probability of every token; `Answer::probability_of(key)` is how sure the model was of the string it wrote for `key`.
- Sends `model`, `temperature`, `max_tokens`, `logprobs: true`, `messages: [system, user]`, and
  `response_format: {"type": "json_schema", …, "strict": true}`, so vLLM only lets the model write tokens that keep the
  answer valid for the schema. A bearer token only when `LLM_API_KEY` is set.
- `VllmError`: `Timeout`, `Transport`, `ContextWindowFull` (a prompt plus `max_tokens` longer than the context, vLLM's
  400), `BadResponse`, `Malformed` (an answer cut off at `max_tokens` shows up as this). Every one of them is a failed
  turn, not an HTTP error.
- `[llm_settings]` in `app/config/settings.toml`: `timeout_seconds = 120`, `temperature = 0.0`, `max_tokens = 256`,
  shared by all machines.

---

## Machines and prompt building (`domain/machine.rs`)

`Machine { role, rules }` and `query::<T>(llm, input, call_session)`. The system message:

```xml
<role>…</role>
<rules>
  <rule>Set `field` to: <valid_value_description>[. Use only a value listed in <allowed_values><field>]</rule>  ← per output field
  <rule>…the machine's own rules…</rule>
</rules>
<allowed_values><caller_intent>
  <allowed_value><value>confirm_yes</value><description>…</description></allowed_value>…
</caller_intent></allowed_values>                                    ← strict fields only
<json_output_format>{ "machine_reasoning": "string", "caller_intent": "string" }</json_output_format>
<conversation_history><call_turn>…</call_turn></conversation_history>  ← omitted when empty, the last 5 turns
<context>
  <current_time>…</current_time><form_state>{json}</form_state><language>Russian</language>
  <response_context>…</response_context>                             ← formulators only, LAST
</context>
```

The user message is `<utterance>…</utterance>`.

- **Context** is `CallSession::context()`: the turn's own context (`current_time`, and `response_context` once the
  intent handler wrote it), `form_state` while a form is active, and `language`. It is sorted by key, so
  `response_context` sits after the long `form_state`, next to the utterance. The 7B model follows it poorly anywhere
  else.
- **`<recently_completed_<form>_form>`**, e.g. `<recently_completed_doctor_appointment_form>`, is the
  `values_summary()` of `HintMap::last_filled_out_form`. `ContextExtractedHandler` adds it to the turn's context in the
  main menu, once the hints are merged (`CallSession::add_recently_completed_form_context`), so the main menu intent
  matcher and response formulator see it and the context extractor does not. With it in its context the extractor
  wrote the form's values as hints on turns that said none of them (a weather question gave a name, a date of birth
  and an appointment date), and the next form was prefilled with them. Form turns never get it.
- **Escaping:** `role`, `rules` and the descriptions of allowed values are verbatim, so they can name other sections by
  tag. Everything else (utterance, history, context, values) is escaped for `<`, `>` and `&`. A consequence: a tag
  written inside `response_context` arrives as `&lt;…&gt;`, so those texts must not refer to tags.
- **The intent descriptions** the matchers see are `Described for CallerIntent` in `call.rs`. Each flow lists its own
  intents in `allowed_values()` of its `Intended…Action` newtype.
- **`<json_output_format>`** comes from schemars, in struct order (`serde_json/preserve_order`). An `Option` field is
  shown with its own type (`"string"`), never as `["string", "null"]`, which the model copied as an array.
- **The JSON schema** vLLM holds the answer to (`json_schema()`): the output's fields and no others, no empty strings,
  only the allowed values of a strict field, and `null` where `ValueSchema::accepts_null` says so. Only `HintMap` uses
  that: without it the model wrote the words "null" and "string" as values. `OutputFormat::fit_schema(schema,
  call_session)` is called with that schema before it is sent, for what the call's state says the answer can be. Only
  `ExtractedFormValue` uses it (see "Decided in code", 9). It changes the schema only, the prompt says nothing of it.
- **Logging:** every call logs `Machine prompt` (full system and user XML) and `Machine answered` at `info`.
- **Adding an output type:** a newtype per field implementing `ValueSchema`, a struct deriving
  `JsonSchema, Deserialize, Default` implementing `OutputFormat::iter_schemas` (keys = serde names).
  `OutputFormat::read_answer` is for what the answer tells besides its JSON, such as `confidence`.

---

## Main menu (`main_menu_flow`)

| Machine | Output | Job |
|---|---|---|
| `Machine::main_menu_context_extractor()` | `HintMap`: `caller_full_name`, `patient_full_name`, `date_of_birth_iso_8601`, `appointment_spoken_date`, `appointment_spoken_time`, each a string or null (not `last_filled_out_form`, see Hints) | What the utterance already says for a later form. The spoken date and time are kept in the caller's words |
| `Machine::main_menu_intent_matcher()` | `ExtractedMainMenuIntent`: `machine_reasoning`, `caller_intent`, plus `confidence` from the token probabilities | One intent of the main menu |
| `Machine::main_menu_response_formulator()` | `FormulatedResponse`: `spoken_response` | The sentence the caller hears, from `<response_context>` |

The menu: `unsupported`, `get_information[calendar_help]`, `get_information[get_current_uah_per_eur]`,
`get_information[get_current_weather_in_berlin]`, `start_form[doctor_appointment]`, `repeat`, `end_call`,
`transfer_to_human`.

| Intent | `main_menu_intent_context_handler` |
|---|---|
| `get_information[…]` weather, rate | fetched (5 s timeout); on failure the formulator is told to apologize |
| `get_information[calendar_help]` | fixed text: say sorry, dates cannot be helped with |
| `start_form[doctor_appointment]` | `start_form`: builds the form, prefills it from the `HintMap`, state → `FormInProgress`, `Started the … form.` plus the next step |
| `repeat`, when there is history | "The caller asked to hear your last reply from `<conversation_history>`." |
| `end_call` | action `end_call`, a short goodbye |
| `transfer_to_human` | action `transfer_to_human` |
| `unsupported`, and anything else | says so and lists what is offered: the menu without `unsupported`, `calendar_help`, `repeat`, `end_call`, `transfer_to_human` |

### Hints

`HintMap` is kept for the whole call in `call_memory.hint_map`. Every main menu turn merges what its utterance said
into it (`merge`: a new value replaces the old one, a missing one keeps it). Form turns do not touch the hints, except
the turn that completes a form (see Form completion).

`last_filled_out_form: Option<Form>` is the one key that is not a hint: the form the caller completed last in the call,
whichever form it is. `IntentMatchedHandler` sets it once the reply of the completing turn is formulated, next to the
`FormCompletedEvent`, so a failed turn sets nothing, and it is saved with the session. It is `#[schemars(skip)]`: the
context extractor's `<json_output_format>` and JSON schema do not have it, and `merge` leaves it alone.

When a form is started, `HintMap::prefill` puts the hints into it. Those values wait for the caller's confirmation like
values given inside the form, so the caller is only asked whether they are right. For the doctor form:

| Field | From |
|---|---|
| `patient_name` | `patient_full_name` |
| `date_of_birth` | `date_of_birth_iso_8601` |
| `appointment_date` | `appointment_spoken_date` |
| `appointment_time` | `appointment_spoken_time` |

A hint the field's kind cannot read, or the field's validators refuse, is left out and the field is asked for.
`caller_full_name` is not used by any form. The completion callback of the doctor form writes the same four fields
back into the same four hints.

---

## Form (`form_flow`)

| Machine | Output | Job |
|---|---|---|
| `Machine::form_context_extractor()` | `ExtractedFormValue`: `form_field` and `form_field_value`, both optional | The one field and value the utterance gives. A date as `YYYY-MM-DD`, a spoken date or time as the caller's words for the day or the time of day in English and lowercase. Nothing for a plain yes or no, a question or a request. A value changed only in part is written whole ("no, the tenth" → the full date) |
| `Machine::form_intent_matcher()` | `ExtractedFormIntent`: `machine_reasoning`, `caller_intent`, plus `confidence` | One intent of the form menu |
| `Machine::form_response_formulator()` | `FormulatedResponse`: `spoken_response` | The sentence the caller hears, from `<response_context>` |

The form menu, in the order the matcher sees it: `unsupported`, `provide_form_field_value`,
`correct_form_field_value`, `refer_to_context_for_form_field_value`, `confirm_yes`, `confirm_no`, `cancel_form`,
`get_information[form_information]`, `repeat`, `end_call`, `transfer_to_human`.

Weather, exchange rate, `calendar_help` and `start_form` are not in it. Asked inside a form they are `unsupported`: the
caller is told that this cannot be helped with right now and the pending question is asked again. With `calendar_help`
in the menu, "в следующую субботу" said as the appointment date was matched as a calendar question (confidence
0.9998), so the form could not be finished.

### What each intent does (`form_intent_context_handler`)

| Intent | The form | The formulator is told |
|---|---|---|
| `provide_form_field_value`, `refer_to_context_for_form_field_value` | The value is parsed with the field's kind, shown to the field's validators and recorded for the field the extractor named, or for the current one. The field then waits for confirmation. A value for a later field accepts the one read back for the current field | `Recorded "…" for <field>.`, or that the value could not be understood, or that no value was given, or `"…" was not recorded for <field>.` plus the validator's message |
| `correct_form_field_value` with a new value | The same, but it never accepts the value read back, whichever field it is for | The same |
| `correct_form_field_value` without a new value | As `confirm_no` | As `confirm_no` |
| `confirm_yes` | The current field is confirmed. With another value for the field that was read back ("yes, but it's 1992"), that value replaces it and waits for confirmation | `The caller confirmed <field>. Thank them.` On the last field: that the form is complete, with all values, and to ask whether the caller needs anything else |
| `confirm_no` | The current field's value is dropped and its rejection counter goes up | That the value was wrong and dropped, and to apologize. From the third rejection of one field: to apologize and offer a human |
| `get_information[form_information]` | Unchanged | To answer in one sentence and say the next step in a sentence of its own |
| `repeat` | Unchanged | To say the last reply once more, with the reply quoted |
| `cancel_form` | Dropped, the call is back in the main menu | That the form was cancelled, and to ask whether the caller needs anything else |
| `end_call` | Dropped, the call is back in the main menu, action `end_call` | A short goodbye |
| `transfer_to_human` | Kept, action `transfer_to_human` | To say the caller is being transferred |
| `unsupported` and anything else | Unchanged | That this cannot be helped with while the form is being filled in, and not to answer it |

While the form is still open, the next step is added at the end of the response context, except after `repeat` and
`transfer_to_human`: `Next, ask the caller for: <description>.` or
`Next, ask the caller to confirm that <description> is <value>.`

A yes or no when nothing is waiting for confirmation changes nothing, and the pending question is asked again. The one
exception is a yes-or-no field that has no value yet: there the yes or no is taken as its value. The doctor form has no
such field.

### Decided in code, not by the model

1. **The extracted value only counts when the intent fills a field.** The extractor writes a value the form already
   knows on many "yes", "no" and question turns, so its value is carried next to the intent
   (`IntentMatched::Form(intent, value)`) and used only where the intent fills.
2. **A value the field already holds is not a value the caller gave** (`holds`). Without this a plain "нет" that the
   matcher took for a correction recorded the old name again.
3. **A correction without a value is a rejection.**
4. **A value for a later field confirms the current one** when that one was waiting for confirmation: the caller moved
   on from the value that was read back. Only for `provide` and `refer`, never for a correction.
5. **An agreement with another value is a correction** (`replaces_read_back`). It has to be sure, because the extractor
   also writes the held value once more in its own words: it named the field, the field's kind can read the value, and
   for a text the new words were said in the utterance, for a date or a number the value differs. Never for a yes-or-no
   field.
6. **Another value in place of the one that was read back says that one is wrong**, even when the new one cannot be
   read or a validator refuses it. The read-back value is dropped and the field is asked for again.
7. **A value a validator refuses is not recorded** (see Validators).
8. **A completed form is sent only after its reply was formulated**, so a turn that fails sends nothing.
9. **`form_field` is held to the fields the caller can be answering** (`Form::answerable_fields`, put into the JSON
   schema as an `enum` by `ExtractedFormValue::fit_schema`). While the current field has no value the caller was
   asked for it, so the fields after it are not offered; once its value is read back, every field is. Without it
   the extractor filed a day said with a clock time under the time while the day was asked for ("Morgen um 15 Uhr",
   "Tomorrow at 3 pm"), and a bare "Morgen" too, so the day was asked for again and the call went in circles.

### When a turn fails

`CallSession` remembers the state the turn found (`state_before_turn`). `fail_turn` puts that state back, sets the
action to `continue` and saves a fixed apology as the reply, in German, Russian, Ukrainian or English:

- main menu: the apology plus what the assistant offers;
- form: "Sorry, an error happened on our side, please say that again." The form is as it was, so the caller only has
  to say it again.

---

## Forms (`domain/form.rs`)

- `Form { kind, fields }`, `FormField { name, description, kind, value, state, confirmation_failed_counter }`.
  A field is a step: `Queued` → `AwaitingConfirmation` → `Completed`; the current field is the first not completed.
- Methods: `field` (builder), `current_field`, `find_field`, `is_filled`, `is_ahead`, `answerable_fields`,
  `fill_field`, `confirm_current`, `reject_current`, `values_summary`, `context_value` (the `form_state` JSON:
  `{"form", "current_field", "fields"}`).
- Kinds and `parse`: `string` (non-empty), `unsigned_integer`, `integer`, `float` (comma = decimal point), `bool`
  (true/yes, false/no), `date` (ISO `YYYY-MM-DD`, `time` crate), `spoken_date` and `spoken_time` (non-empty text kept
  as said, for the receiving project to work out). Values serialize externally tagged, e.g.
  `{"spoken_date": "next saturday"}`, `{"spoken_time": "in the morning"}`.
- **No two neighbouring steps may take the same kind of value** (`FormFieldKind::is_like`: both dates, both numbers,
  or equal): an answer meant for one fits the other. The builder asserts it, and the test `every_form_builds` builds
  every form so a bad order fails a test, not a call.
- `DoctorAppointment`: `patient_name` (string), `date_of_birth` (date), `reason` (string), `appointment_date`
  (spoken_date, "Date of the appointment"), `appointment_time` (spoken_time, "Preferred time of the
  appointment"). A spoken date and a spoken time are not alike for `is_like`, so the two can be neighbours.

### Validators

A validator is an async function that gets the app state and the value a field is about to take:

```rust
async fn date_of_birth_not_in_the_future(_: &AppState, value: &FormFieldValue) -> Result<(), ValidationError> {
    …
    Err(ValidationError("A date of birth cannot be in the future. Tell the caller that.".to_string()))
}
```

- `ValidationError(String)` is written for the response formulator: what to tell the caller. The flow puts it into
  `response_context` as `"<value>" was not recorded for <field>. <message>`, and the next step that follows asks for
  the field again.
- **Which field has which validators** is the `match (self, field)` in `FormSupported::validate`, under `build()`. To
  add one, write the function and add an arm. Several for one field: `first(app_state, value).await?;
  second(app_state, value).await`.
- They are found by the field's name and are not kept in `FormField`, because the form is saved in Redis between the
  turns, shown to the machines as `<form_state>` and POSTed when it is complete. **A renamed field has to be renamed in
  `validate` too**, as in `HintMap::form_values`. Nothing fails when it is forgotten: the validator just stops running.
- They run in two places: `validate_and_fill` (every value a turn fills in) and `HintMap::prefill` (hints, where a
  refused one is left out without a word).
- `AppState` reaches them with the event: `CallerSpokeEvent`, `ContextExtractedEvent` and `IntentMatchedEvent` carry an
  `Arc<AppState>` next to the call's session, because the handlers are built before the app state, as a part of it.
- A yes or no taken as the value of a yes-or-no field (`confirm` / `reject`) is not shown to validators.
- The only one so far: `date_of_birth` cannot be after today, on the clock `current_time` is taken from.

### Why relative dates are kept as said

Tested on 2026-09-26 by replaying the logged prompt on 8 phrases: the 7B model got 1 right when asked for
`YYYY-MM-DD` ("next Saturday" → a Wednesday), 0 of 6 with a 15-day calendar to look them up in, and still did
arithmetic when asked for relative words. Copying the caller's words worked for 6 of 8. So the appointment date and
time are stored as said; the receiver interprets them against `completed_at`. A read-back never names a computed date.

---

## Form completion → another project

- When the last field is confirmed, `form_intent_context_handler` returns the form and sets the state to `Idle`. Once
  the reply is formulated, `IntentMatchedHandler` logs `Completed FORM`, calls the form's completion callback, sets
  `last_filled_out_form` and dispatches `FormCompletedEvent { call_id, form }`.
- **The completion callback** is `FormSupported::on_completed(hint_map, form)`: it gets the call's `HintMap` and the
  completed form and says what of the form goes back into the hints. Which form has which callback is the `match` in
  `on_completed`, under `validate()`; the callbacks are at the bottom of `form.rs`. Like the validators it is found by
  the form's kind and not kept in the form. Everything that saves comes after it: `last_filled_out_form`, the POST
  and the session. `doctor_appointment_completed` writes `patient_name`, `date_of_birth`, `appointment_date` and
  `appointment_time` into `patient_full_name`, `date_of_birth_iso_8601`, `appointment_spoken_date` and
  `appointment_spoken_time`, so a second form in the same call starts with them, each waiting for the caller's yes.
  `reason` has no hint.
- `FormSubmitter` POSTs, in a spawned task (10 s timeout), to `FORM_SUBMIT_URL`:
  ```json
  {"call_id": "…", "completed_at": "2026-10-06T09:51:02.55+03:00", "form": {"kind": "doctor_appointment", "fields": [ … ]}}
  ```
  Logs `Form submitted` or `Could not submit the form`; with no URL it logs a warning and sends nothing.
- `FORM_SUBMIT_URL` is in `.env` and passed by `docker-compose.yml`. From the container, a service on the host is
  `http://host.docker.internal:<port>/…`.
- No retry. `call_id` is in the payload so the receiver can deduplicate.

---

## Call session and storage (`domain/call_session.rs`)

- `CallSession { call_id, call_turn_context, call_turn_outcome, data }`. Only `data` (`CallData`) is kept between the
  turns: `language`, `state` (`CallState`), `call_memory` (`conversation`, `hint_map`), `last_spoken_response`.
- `conversation` holds the last `MAX_CONVERSATION_TURNS` (5) turns, which is 10 messages: `save_call_turn` drops the
  oldest ones, so that is also all `<conversation_history>` shows the machines.
- Redis key `call_id.<id>`: `CallData` as MessagePack, TTL `call_session_ttl_seconds` (3600), saved on every request.
  A value that cannot be decoded, such as one written by an older shape of the type, counts as a miss and the call
  starts over.
- `call_turn_context` lasts for the turn: `current_time`, e.g. `"Tuesday, 2026-10-06 09:25 +03:00"`, in the
  container's clock (`TZ` from `APP_TIMEZONE`), and `response_context`.
- `call_turn_outcome` (`CallTurnOutcome`) lasts for the turn too: `caller_intent`, `machine_reasoning`, `confidence`,
  `action`. It is what the HTTP response is built from.
- **Language:** `CallerSpokeHandler` sets the call's language from every utterance lingua can tell the language of. A
  new call starts in `call_settings.default_language` (`de`). `language` in the request is only logged.
- **Per-call lock:** `AppState.call_locks` (`CallLocks`) gives one `tokio::sync::Mutex` per call id. The middleware
  takes it before loading and holds it until after saving, so the next turn of the call loads what this one saved;
  other calls are not blocked. In-process only. The prompt loop does not use it.

## Event system (`event/`)

Handlers are async (`EventHandler<E>::handle(&self, &mut E, &Dispatcher) -> impl Future`) and may dispatch nested
events, which is how a turn runs: `CallerSpokeEvent` → `ContextExtractedEvent` → `IntentMatchedEvent` →
`FormCompletedEvent`. Handlers are built once at startup, so what belongs to a call or to the running app comes with
the event: the session as `Arc<RwLock<CallSession>>`, the app state as `Arc<AppState>`. A handler releases the session
lock before it dispatches the next event.

## HTTP API

- `POST /assistant/handle-request` (needs `x-call-id`), `GET /mimic-client`, `GET /version`, `GET /test-session`
  (still panics: it expects a `UserSession` nothing adds; left alone on purpose). Only the assistant route is behind
  the call-session middleware.

  | Key | From |
  |---|---|
  | `answer` | `data.last_spoken_response` |
  | `selected_function`, `reasoning`, `confidence` | `call_turn_outcome`, written by the intent handler of the flow the call is in. `null` when the turn failed before an intent was matched |
  | `language_detected` | the call's language as an ISO 639-1 code |
  | `action` | `continue`, `end_call` or `transfer_to_human` |
  | `response_context` | `call_turn_context["response_context"]`, what the intent handler gave the response formulator. `null` when the turn failed before it |
  | `hint_map` | `data.call_memory.hint_map` once the turn is over, every key, `null` for a hint the caller did not give. `last_filled_out_form` in it is a whole `Form` or `null` |
  | `form` | the `Form` of `CallState::FormInProgress` once the turn is over, `null` in the main menu. So also `null` on the turn that completes or cancels a form |

- **A failed turn is a 200 with the apology as `answer`**, not a 502 or 504. `ApiError::Inference` is therefore never
  produced; it is still in `error.rs`. Bad requests (no `x-call-id`, empty `request_text`) are 400 with `{"error": …}`.

## Mimic client (`client/index.html`)

- One call id per page load (`crypto.randomUUID()`), shown on the page and sent as `x-call-id`; reload = new call.
- Acts on `action` after the answer is read out: `end_call` hangs up, `transfer_to_human` too (nothing to transfer to).
- Shows the rest of the response under every answer: the short keys as badges (`confidence` as a percentage with a
  meter), then `reasoning` and `response_context` as lines, `hint_map` as rows and `form` as one row per field with its
  value and state, the current field marked. `hint_map.last_filled_out_form` gets the same rows as `form`, in a block
  of its own. A key the page does not know is still a badge.
- The newest line is on top of the transcript, so an answer sits above the sentence it answers.

---

## Configuration and environment

- `.env` / `.env.dist`: `APP_TIMEZONE=Europe/Kyiv` (compose passes it as `TZ`), `FORM_SUBMIT_URL=`. `.env.stdin` has
  the localhost values for the prompt loop and a local server, `.env.test` the ones for `cargo test`.
- **GPU memory:** `LLM_GPU_MEMORY_UTILIZATION=0.80`. At 0.90 vLLM left the desktop too little of the 12 GB; opening a
  browser page then made Windows page vLLM's memory out and generation fell to <1 token/s. A change only applies when
  the container is recreated (`docker compose up -d vllm`), not on a restart.
- Cargo builds on this machine go to the shared `H:/cargo-target` (global `~/.cargo/config.toml`); a scratch copy of
  this workspace built there overwrites this tree's artifacts. Use a separate `CARGO_TARGET_DIR` for scratch builds.
- On Windows a running `app.exe` cannot be replaced: stop it before `cargo build` or `cargo run`.

## How to run

```bash
./update-app.sh                          # builds the app image, then recreates only the app container

# prompt loop, from the repo root (reads .env, then .env.stdin, and app/config/settings.toml)
cargo run -- --prompt-loop

# the HTTP server against the local services
cd app && set -a && . ../.env.stdin && set +a && APP_PORT=8089 cargo run

curl -s -X POST http://localhost:8080/assistant/handle-request -H 'Content-Type: application/json' \
  -H 'x-call-id: test-call-1' -d '{"request_text": "I would like to book a doctor appointment"}'
```

- Mimic client: http://localhost:8080/mimic-client
- Logs: `./logs/app.log.<date>` (UTC). Search `Machine prompt`, `Machine answered`, `Completed FORM`, `Form submitted`.
- `cargo check -p app --all-targets` (31 warnings, see Leftovers). `cargo test -p app domain::` runs the 9 tests that
  need no services; the five in `tests.rs` are about the config, Postgres and Redis.
- `evals/` holds the DeepEval suites that play goldens against the running app, see `evals/README.md`.
- A piped script for the prompt loop has to end with `exit`: at the end of the input it keeps reading empty lines.

---

## Verification

**Refactor, 2026-10-05** (details in `updates.md`). Scripted conversations through the prompt loop against the local
vLLM and Redis, English and Russian. With the final prompts English was right in every turn (3 conversations, 40
turns). Over the 23 conversations run after the form menu was narrowed, 7 of 289 form turns with a known expected
intent were wrong, all in Russian, and 8 of 290 form replies had Chinese in them, all in Russian.

**Hints and corrections, 2026-10-06.** Replays of 239 confirmation answers (EN and RU) over 9 form states:

- The matcher labelled every "yes, but X" / "no, X" as `correct_form_field_value` (59 of 59). Rewording `confirm_no`
  to "A denial that gives no other value…" made plain denials taken for a correction fall from 30 to 12 of 65.
- The weak spot is the extractor's value: fragments ("1992"), denial words as the value, and the held value written
  again on a plain yes. The guard in `replaces_read_back` misfired 0 times; "yes + any other value" misfired 8 to 12.
- Letting the `HintMap` keys be null in the JSON schema took the junk values ("string", "null") from 7 to 13 in 26
  calls to 0 in 42.

**Validators and the merged hint, 2026-10-06.** `cargo test -p app domain::`: 7 pass. Scripted calls over HTTP against
the app container, and two through the prompt loop. "Right" means the intent, the form's state and the response
context:

| | Calls | Result |
|---|---|---|
| English, a date of birth in the future, first as an answer and then as "no, it is 2092" after a read-back | 10 | Refused and explained in every call ("The date of birth cannot be in the future. Please provide a valid date of birth."), the read-back value dropped. In 1 call the date said again after the second refusal was matched as `confirm_yes`, see gap 6 |
| German, the same with full-sentence answers ("Er ist am 13. Juni 2091 geboren") | 3 | Right in every turn, in German ("Leider kann das Geburtsdatum nicht in der Zukunft liegen. Bitte geben Sie ein gültiges Geburtsdatum an.") |
| German, the same with bare dates ("13. Juni 2091") | 4 | State right, replies wrong: every bare date was detected as English, see gap 1 |
| Russian, the same with the year in digits | 1 | Right in every turn, in Russian. Two replies added the patient's age unasked |
| Date and time given before the form: "next saturday in the morning", "morgen um 15 Uhr", "на завтра на три часа дня" | 3 | Merged, read back at the end and submitted as `next saturday in the morning`, `morgen 15 Uhr`, `завтра три часа дня` |
| A date of birth in the future given before the form (prompt loop) | 1 | Left out, the field was asked for |

A hint is kept in the caller's language, while a value given inside the form is written in English by the form
extractor. With the year in Russian words ("две тысячи девяносто первого") the extractor wrote 1991, so there was
nothing to refuse.

**Completion callback, two appointment fields and the held field name, 2026-10-06.** `cargo test -p app domain::`:
9 pass. `evals/run.py -r 3` against the app container (474 test cases, 741 turns), next to the run before the change,
which had one `appointment_datetime` field (426 test cases):

| Check | Before, de | After, de | Before, en | After, en |
|---|---|---|---|---|
| Form State | 351/366 (96%) | 416/423 (98%) | 144/147 (98%) | 177/180 (98%) |
| Completed Form | 15/15 | 13/15 | 9/9 | 9/9 |
| Intent | 383/393 (97%) | 430/441 (98%) | 174/183 (95%) | 207/216 (96%) |
| Reply Asks | 380/453 (84%) | 429/507 (85%) | 174/192 (91%) | 210/225 (93%) |

- Date and time said before the form fill both fields, are read back one after the other, go back into the hints
  when the form is complete, and a second form in the call starts with them.
- **Day and time in one answer while the day is asked for.** Without the held field name the extractor filed 8 of
  12 such answers under the time ("Morgen um 15 Uhr", "Tomorrow at 3 pm", "Monday at ten"), and a bare "Morgen" too,
  so the day stayed empty and two scripted calls went in circles. Ten wordings of the extractor's rules and
  descriptions changed nothing: 35 to 38 of 48 probes right, against 36 without them. With `form_field` held to the answerable fields, 45
  of 48, and the day alone is written ("morgen", "tomorrow", "montag"). Replaying the 196 logged extractor prompts
  with it changed 9 answers, none for the worse: the day-with-time answers, and "Irgendwann im Sommer" as a date of
  birth, which used to be filed under the appointment. A time alone said while the day is asked for is now recorded
  as the day and read back.
- **Still open:** the formulator reads back the day and the time although only the day was recorded ("Der Termin
  ist übermorgen um neun Uhr. Ist dies die korrekte Zeit?"), and the caller's yes confirms the day only. In the German
  call that says "Übermorgen um neun Uhr", the time said again was matched as `confirm_yes` in 2 of 3 runs and the
  form was not completed. The English one ("Tomorrow at 3 pm") completed in 3 of 3.
- **"Preferred date of the appointment"** as the field's description made German replies ask for "den bevorzugten
  Termin für den Termin": of 47 replayed German replies of the date step, 38 asked for the date, and 46 with "Date of
  the appointment". "Preferred time of the appointment" asked for the time in 42 of 42.
- **Reading the suite's pass rates:** the goldens of the form suite start from one walk through the form per language
  and run, with its replies as their history, so the goldens of a step pass or fail together. German goldens outside
  the known gaps were 84% in one run and 75% in the next, with only the date field's description changed: ten goldens
  of the first three steps had flipped together. Nine of them had flipped the other way between the two runs before,
  whose prompts for those steps were the same. Two runs do not rank two wordings; the replays above do.

**How wordings are compared:** one run says little. vLLM does not always give the same answer at temperature 0 when two
answers are close, and one different reply changes the history of every turn after it. Replay every logged
`Machine prompt` of one kind with the change applied and count.

## What the 7B model does badly (learned by replay)

- Any date arithmetic, with or without a calendar in the prompt.
- Follows `response_context` poorly when it is not the last thing in `<context>`; repeats its previous question.
- Copies type hints literally (`["string", "null"]` → arrays) and writes a word such as "null" for a missing value
  unless the schema lets it write null.
- Takes a missing year from other dates in the history and follows examples in rules too eagerly: examples in the hint
  extractor's rules were copied into its output.
- Drifts into Chinese inside Russian replies, and its confidence does not point at its mistakes (two wrong intents had
  0.9989 and 0.9998).

---

## Known gaps and next steps

**Language**
1. **The call's language follows every utterance, and short answers are detected wrongly.** A bare German date
   ("13. Juni 1991") was detected as English in 4 of 4 calls, short Russian answers ("голова болит") as Ukrainian. The
   reply then comes in that language. After a refused date of birth it is worse: the next, valid date got the refusal
   sentence again in all 4 German calls, although the form had recorded it. With full-sentence answers the same call
   was right 3 of 3. Ways out: `with_minimum_relative_distance` on the lingua builder in `classifier.rs`, so an unsure
   detection keeps the call's language, or only switching the language on longer utterances.
2. **Chinese inside Russian replies** of the form formulator (8 of 143 Russian replies on 2026-10-05). A `pattern` on
   `spoken_response` in the JSON schema, allowing Latin, Cyrillic, digits and punctuation, turned all 8 into correct
   Russian on replay. Not applied. `updates.md` has the pattern.
3. Confirmation questions in Russian sometimes leave the value out ("Полное имя пациента верно?"), a read-back named
   another day than the value held, and the English field description got into a Russian reply ("чтоPreferred date and
   time of the appointment"). The read-back is worded by the formulator; one rendered in code
   (`// TODO: add readback trait` in `main.rs`) is the next lever.
4. Some Russian names come out transliterated in hints.

**Dialogue**
5. The start-of-form reply can read back everything the caller said, not only the field it was told to confirm, and
   the caller's "yes" then confirms only that one field. With a prefilled appointment, Russian replies said the
   appointment was booked before the form was complete.
6. **A value matched as `confirm_yes` while its field has no value is lost.** After "no, it is 2092" was refused and
   the read-back value dropped, "June 13th 1991" was matched as `confirm_yes` in 1 of 6 English calls. Nothing was
   recorded, the reply read the date back anyway, and the caller's "yes" had nothing to confirm.
7. `repeat` in the main menu tells the formulator to repeat its reply "from `<conversation_history>`", which arrives
   escaped; the form flow quotes the reply instead. Not changed in the main menu. Unclear words ("хм", "так", "ну",
   "hmm") are taken for `confirm_yes` in 7 to 9 of 160 matcher calls.
8. "Andrew with a v" is recorded with the words in it.
9. A completed form is kept in `HintMap::last_filled_out_form` and shown to the main menu matcher and formulator, but
   no main menu intent is about it. "What did I book?" / "Für wen habe ich den Termin gebucht?" matched `calendar_help`
   in 4 of 4 calls, and the formulator answered from the form against its `response_context`. On such a question the
   context extractor also fills the hints from `<conversation_history>` (3 of 3 calls: name, date of birth,
   appointment). Since the completion callback the hints hold the form's values anyway.
10. One value per turn: a caller who says the day and the time in one answer is asked for the time again once the
    day is confirmed, and the reply in between reads both back (see Verification). Several values per turn, or a
    read-back rendered in code, would close it. No final confirmation of the whole form; no "end the call anyway?" inside a form; no
    `required` flag; no skip or don't know; no `greeting`, `who_are_you`, `this_call` intents.
11. `end_call` and `transfer_to_human` only set `action` (two TODOs in `main_menu_intent_context_handler`).
12. The menu depends on the flow, not on the state inside it: `confirm_*` is offered with nothing pending.

**Correctness and safety**
13. PII in logs: full prompts with history and form values at `info`, the completed form in `Completed FORM` and in
    `Form was submitted!`, which is logged before the POST is sent.
14. Unbounded history in the prompts and in Redis. A long call ends in `ContextWindowFull`, which is a failed turn.
15. `x-call-id` is trusted as given: ids must be unguessable or authenticated by the telephony side.
16. The call lock is per process; several app instances would race (last write wins).
17. Form submission isn't retried; a failed POST is only logged.
18. Validators are matched by field name and have no test of their own: a test needs an `AppState`, which needs
    Postgres and Redis.

**Leftovers**
19. The 31 warnings: the cookie-session middleware and `session.rs`, `/test-session`, the `db` and cache helpers,
    `AppState` fields nothing reads, the unused parts of the event bus, `llm_system_prompt_file` and
    `session_ttl_days`, `VllmClient::model` and `query_json`, four `AllowedValue` variants, the `call_id` fields of two
    events, unused imports.
20. Prompt-file remnants: `llm_system_prompt_file` in `settings.toml` / `settings.rs`, `LLM_SYSTEM_PROMPT_FILE` and the
    `./app/prompts` mount in `docker-compose.yml`; `app/prompts/` is empty.
21. `docker/classifier` (a GLiNER2 service tried as machine #1): its container still runs, the app does not call it.
22. `app/src/vllm.rs.old`, `bash.exe.stackdump`, `sqlx::migrate!` commented out in `main.rs`.
23. `docs/` was last changed on 2026-10-02, before the refactor.

---

## Still true from 2026-09-17

- Logging: stdout plus daily rolling files via `tracing-appender` in `LOG_DIR` (`/var/log/app` → `./logs`); filter
  `RUST_LOG=info,sqlx=warn,tower_http=info`; settings load before logging starts.
- LLM settings: 120 s timeout because a cold/contended GPU took 30 s+; `max_tokens` capped because an uncapped
  repetition loop ran to the 4096-token context.
- `serde_json` `preserve_order` is enabled workspace-wide.
- Earlier research (`git show fd90a3d:context.md`): record where a slot value came from; idempotent commits; hand off
  after N failed turns; order prompt layers static → volatile (helps vLLM's prefix cache).

## Working agreements

- Keep it stupid simple; do only what the current step asks.
- Follow the existing style: a newtype + `ValueSchema` per machine-output field, `/* --- */` separators, few comments.
- The user commits; edits they make between steps are deliberate.
- When the tree doesn't compile for unrelated reasons, verify in a scratch copy (with its own `CARGO_TARGET_DIR`).
- Research established practice before designing a dialogue mechanism; compare prompt wordings by replaying many
  logged calls, in Russian as well as English.
