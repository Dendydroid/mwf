# Context — ai_assistant

**Updated:** 2026-10-06 · **Repo:** `H:\rust\ai_assistant` (the vocabulary file, the replies of a form worded by
code, the language rule for short utterances and the agreement checker were committed together, after `fd64ab4
questionable state of things`) · **Stack:** Rust / axum 0.7 ·
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
3. **Response formulator** words a reply, where one needs words of its own.

After the second, Rust code changes the call's state and decides what the caller hears. **The model extracts, code
decides.** In a form code also words the reply: a sentence of the vocabulary for what the turn did, then the question
of the step the form is on. The formulator words the main menu's replies and the answer to a question about the
form. So a main menu turn is three model calls and a form turn two, a little under one second on the local GPU.

The form has a fourth machine, the **agreement checker**. It is only asked when the form's matcher took the
utterance for a yes, it is shown that utterance and nothing else of the call, and it says whether the caller asks a
question. **A question is never a yes**: it is then handled as a question about the form. That is a third call of
about 90 ms on a turn the matcher took for an agreement.

**No text is in the code.** Every sentence said to a caller and every text shown to a machine is in
`app/config/llm_vocabulary.toml`, and the code finds it by a key (see Vocabulary).

Supported: current weather in Berlin, EUR→UAH rate, one form (doctor appointment) filled field by field with each
value confirmed and checked by validators (one answer can give several values), questions about the form in progress
and about the one filled out last, a refusal for calendar questions, repeat, end call, transfer to a human, and a
polite refusal listing what is supported. A completed form is POSTed to another
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
       CallerSpokeHandler        1. lingua detects the language of an utterance of three words or more and
                                    sets the call's language. A shorter one keeps the language the call has
                                 2. the flow's context extractor
                                      main menu → HintMap
                                      form      → ExtractedFormValues (a value or null per field of the form)
       ContextExtractedHandler   main menu: merges the hints into the call's HintMap
                                 the flow's intent matcher → ExtractedMainMenuIntent | ExtractedFormIntent
                                 form, and the matcher chose confirm_yes: the agreement checker, asked with the
                                   utterance alone → CheckedAgreement. A question becomes
                                   get_information[form_information], with no confidence
       IntentMatchedHandler      the flow's intent context handler changes the state and returns the Reply:
                                   Said(text)         code worded it from the vocabulary: every turn of a form
                                                      but one, and the main menu turn that starts a form
                                   Formulated{then}   the flow's response formulator words it from
                                                      response_context, and code says `then` after it: the
                                                      main menu, and a question about the form
                                 → save_last_exchange
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
| `app/config/llm_vocabulary.toml` | Every text: what code says to the caller in de, en, ru and uk, and what the machines are shown, in English |
| `app/src/vocabulary.rs` | `vocabulary()`, the global every text is found through. `Vocabulary` and its parts as the file has them, `Phrase`, `Localized` (a text per language), `fill_in`, `Vocabulary::check()` |
| `app/src/domain/flow.rs` | At the top what both flows share: `FlowContext`, `IntentMatched`, `Reply`, `Reasoning`, `SpokenResponse`, `FormulatedResponse`. Then `main_menu_flow` (`HintMap`, its machines, `main_menu_intent_context_handler`, `start_form`) and `form_flow` (its machines, `ExtractedFormIntent` with `is_agreement` / `as_question`, `CheckedAgreement`, `FormulatedAnswer`, `FormTurn`, `form_intent_context_handler`, `fill` / `confirm` / `reject`, `next_step`) |
| `app/src/domain/form.rs` | `Form`, `FormField`, `FormFieldKind` (+ `parse`, `is_like`), `FormFieldValue` (+ `spoken`), `StepState`, the form schemas in `FormSupported::build()`, `FormSupported::vocabulary()`, the validators in `FormSupported::validate()`, `ValidationError`, the completion callbacks in `FormSupported::on_completed()` |
| `app/src/domain/machine.rs` | `Machine`, `query`, `query_utterance_alone`, XML prompt rendering, the JSON schema sent to vLLM, `ValueSchema` / `OutputFormat` / `AllowedValue` / `Described` |
| `app/src/domain/call.rs` | `CallerIntent` (labels, `all`, `from_label`; its descriptions for the matchers are the vocabulary's), `CallAction`, `GetInformationSupported`, `FormSupported`, `flat_enum!` |
| `app/src/domain/information.rs` | `GetInformationSupported::fetch`: open-meteo weather and NBU EUR rate, each put into words with the vocabulary's templates, and the vocabulary's instruction for the other two |
| `app/src/domain/call_session.rs` | `CallSession`, `CallData`, `CallState`, `CallMemory` / `CallTurn` / `Transcript`, `CallTurnOutcome`, `context()`, `save_last_exchange`, `fail_turn`, `CallLocks` / `CallLock` |
| `app/src/event/call_session_loaded.rs` | `CallSessionLoadedEvent`, `InitialContextHandler` |
| `app/src/event/caller_spoke.rs` | `CallerSpokeEvent`, `CallerSpokeHandler` |
| `app/src/event/context_extracted.rs` | `ContextExtractedEvent`, `ContextExtractedHandler` |
| `app/src/event/intent_matched.rs` | `IntentMatchedEvent`, `IntentMatchedHandler` |
| `app/src/event/form_completed.rs` | `FormCompletedEvent`, `FormSubmitter` |
| `app/src/event/events.rs` | registration of all five, `events(&settings, &http, &llm)` |
| `app/src/event/event_bus.rs` | the dispatcher |
| `app/src/classifier.rs` | `detect_language` (lingua: English, German, Russian, Ukrainian), only for a text of three words or more |
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

- **Every text of a prompt is the vocabulary's:** a machine's role and rules (`[machines.<name>]`), the rule each
  output key gets (`[prompt]`), what a key is described as (`[output_values]`), an intent's description
  (`[intents.<label>]`), and what a formulator is told (`[instructions]`, `[facts]`). A machine's constructor in
  `flow.rs` only says which machine it is.
- **Context** is `CallSession::context()`: the turn's own context (`current_time`, and `response_context` once the
  intent handler wrote it), `form_state` while a form is active, and `language`. It is sorted by key, so
  `response_context` sits after the long `form_state`, next to the utterance. The 7B model follows it poorly
  anywhere else.
- **`<form_state>` is in every prompt of a form turn.** The form's formulator is only asked when the caller has a
  question about the form, and the form is what it answers from.
- **One machine is shown the utterance alone**: the form's agreement checker, asked with
  `Machine::query_utterance_alone`. Its system message is its role, its rules and `<json_output_format>`, with no
  `<conversation_history>` and no `<context>`. That is what makes it right (see Verification, "A question must not
  count as a yes"), and its texts must not name a tag its prompt does not have. A test holds both.
- **`<recently_completed_<form>_form>`**, e.g. `<recently_completed_doctor_appointment_form>`, is the
  `values_summary()` of `HintMap::last_filled_out_form`. `ContextExtractedHandler` adds it to the turn's context in the
  main menu, once the hints are merged (`CallSession::add_recently_completed_form_context`), so the main menu intent
  matcher and response formulator see it and the context extractor does not. With it in its context the extractor
  wrote the form's values as hints on turns that said none of them (a weather question gave a name, a date of birth
  and an appointment date), and the next form was prefilled with them. Form turns never get it.
- **Escaping:** `role`, `rules` and the descriptions of allowed values are verbatim, so they can name other sections by
  tag. Everything else (utterance, history, context, values) is escaped for `<`, `>` and `&`. A consequence: a tag
  written inside `response_context` arrives as `&lt;…&gt;`, so those texts must not refer to tags.
- **The intent descriptions** the matchers see are each intent's `description` in the vocabulary, under its label.
  `Described for CallerIntent` in `call.rs` looks it up. Each flow lists its own intents in `allowed_values()` of
  its `Intended…Action` newtype.
- **`<json_output_format>`** comes from schemars, in struct order (`serde_json/preserve_order`). An `Option` field is
  shown with its own type (`"string"`), never as `["string", "null"]`, which the model copied as an array.
- **The JSON schema** vLLM holds the answer to (`json_schema()`): the output's fields and no others, no empty strings,
  only the allowed values of a strict field, and `null` where `ValueSchema::accepts_null` says so. Only `HintMap` uses
  that: without it the model wrote the words "null" and "string" as values. `OutputFormat::fit_schema(schema,
  call_session)` is called with that schema before it is sent, for what the call's state says the answer can be. Only
  `ExtractedFormValues` uses it: its keys are the fields of the form in progress (see "Decided in code", 9), each a
  string or null. `<json_output_format>` is rendered from the schema after that, so it shows the same keys.
- **Logging:** every call logs `Machine prompt` (full system and user XML) and `Machine answered` at `info`.
- **Adding an output type:** a newtype per field implementing `ValueSchema`, with its description as a key of
  `[output_values]`, a struct deriving
  `JsonSchema, Deserialize, Default` implementing `OutputFormat::iter_schemas` (keys = serde names). An output whose
  keys depend on the call, like `ExtractedFormValues`, has no such fields: it is a map, `iter_schemas` is empty,
  `fit_schema` adds the keys and the machine's own rules say what to set them to.
  `OutputFormat::read_answer` is for what the answer tells besides its JSON, such as `confidence`.

---

## Vocabulary (`vocabulary.rs`, `config/llm_vocabulary.toml`)

The code has no text of its own. `vocabulary()` is a global (`LazyLock`) that reads the file the first time it is
asked for, next to `settings.toml` and found the same way (`config/…` from the working directory, `app/config/…`
in the prompt loop). The file is copied into the image with the rest of `app/config`, so a change needs
`./update-app.sh`.

| In the file | What | How the code finds it |
|---|---|---|
| `[phrases.<name>]` | A sentence code says in any form: `thanks`, `sorry`, `offer_human`, `not_understood`, `cannot_help_in_form`, `goodbye`, `transfer`, and the two apologies of a failed turn | `Phrase::Thanks.say(language)` |
| `[validation.<name>]` | What a validator tells the caller | `ValidationError::DateOfBirthInTheFuture.say(language)` |
| `[dates]` | How a date is said: `format` with `{day}`, `{month}`, `{year}`, and the twelve `months` | `FormFieldValue::spoken(language)` |
| `[forms.<form>]` | `started`, `completed`, `cancelled` | `form.kind.vocabulary().started.say(language)` |
| `[forms.<form>.fields.<field>]` | `description` (what the machines read in `<form_state>`), `ask`, `confirm` with `{value}` | `form.kind.vocabulary().field(name).ask.say(language)`, and `next_step(form, language)` for the question of the current step |
| `[intents.<label>]` | `description` for the matchers, and `offer`: how the intent is named when the caller is told what can be helped with | `intent.description()`, `vocabulary().intent(label).offer` |
| `[machines.<name>]` | `role` and `rules`. The form's context extractor also has `examples` per language, and the form's formulator `{language}` in its first rule | `Machine::form_intent_matcher()` and the other six |
| `[output_values]`, `[prompt]` | What a key of a machine's answer is described as, and the rule each key gets | `ValueSchema::valid_value_description`, `render_system_prompt` |
| `[instructions]` | What a response formulator is told to say | `vocabulary().instructions.end_call` |
| `[facts]` | The weather and the exchange rate as sentences, the sky by WMO weather code, the sixteen directions of the wind | `GetInformationSupported::fetch` |

- **A text for the caller has one wording per language** (`Localized`: `de`, `en`, `ru`, `uk`), and every one needs
  all four. `Localized::in_language` matches lingua's `Language` without an arm for "any other", so a language the
  detector is built with later does not compile until the vocabulary has it. A text for a machine is one English
  string.
- **`{name}` in a text** is filled in by `fill_in(text, &[("name", value)])`.
- **Keys are enum variants or field names.** `Phrase`, `ValidationError` and `FormSupported` are written in
  snake_case as serde reads them, an intent by its label (`"get_information[calendar_help]"`), a field by the name
  it has in `build()`.
- **A missing text stops the app when it starts**, not a call. A key the structs name and the file lacks fails the
  read. `Vocabulary::check()`, called in `main` and by a test, looks at what is found by a variant or a name: every
  `Phrase`, every `ValidationError`, every intent's description, every field of every form with `{value}` in its
  `confirm`, twelve months per language. A key the file has and the code does not know fails the read as well
  (`deny_unknown_fields`).
- **Adding a form field:** a line in `build()` and `[forms.<form>.fields.<name>]` with `description`, `ask` and
  `confirm` in four languages. **A sentence code says:** a variant of `Phrase` and `[phrases.<name>]`. **A
  validator's message:** a variant of `ValidationError` and `[validation.<name>]`.
- **Still in the code:** names that are not texts (intent labels, field names, the tags of a prompt), log lines,
  the messages of a panic and of a 400.

---

## Main menu (`main_menu_flow`)

| Machine | Output | Job |
|---|---|---|
| `Machine::main_menu_context_extractor()` | `HintMap`: `caller_full_name`, `patient_full_name`, `date_of_birth_iso_8601`, `appointment_spoken_date`, `appointment_spoken_time`, each a string or null (not `last_filled_out_form`, see Hints) | What the utterance already says for a later form. The spoken date and time are kept in the caller's words |
| `Machine::main_menu_intent_matcher()` | `ExtractedMainMenuIntent`: `machine_reasoning`, `caller_intent`, plus `confidence` from the token probabilities | One intent of the main menu |
| `Machine::main_menu_response_formulator()` | `FormulatedResponse`: `spoken_response` | The sentence the caller hears, from `<response_context>`. Not on the turn that starts a form: that reply is code's |

The menu: `unsupported`, `get_information[calendar_help]`, `get_information[get_current_uah_per_eur]`,
`get_information[get_current_weather_in_berlin]`, `get_information[last_filled_out_form_information]`,
`start_form[doctor_appointment]`, `repeat`, `end_call`, `transfer_to_human`.

| Intent | `main_menu_intent_context_handler` |
|---|---|
| `get_information[…]` weather, rate | fetched (5 s timeout) and put into words with the vocabulary's `[facts]`: `Current weather in Berlin: overcast, 20 degrees Celsius, wind of 6 kilometres per hour from the west-northwest.` On failure the formulator is told to apologize |
| `get_information[calendar_help]` | fixed text: say sorry, dates cannot be helped with |
| `get_information[last_filled_out_form_information]` | Nothing is fetched: `HintMap::last_filled_out_form_information` says to summarize `last_filled_out_form` with its values, only if there is one. With none: that no form was filled out yet, and to summarize nothing |
| `start_form[doctor_appointment]` | `start_form`: builds the form, prefills it from the `HintMap`, state → `FormInProgress`. Code says the reply: the form's `started` sentence and the question of its first step. No formulator is asked |
| `repeat`, when there is history | "The caller asked to hear your last reply from `<conversation_history>`." |
| `end_call` | action `end_call`, a short goodbye |
| `transfer_to_human` | action `transfer_to_human` |
| `unsupported`, and anything else | says so and lists what is offered: every intent of the menu that has an `offer` in the vocabulary, as it is named there ("the euro to hryvnia exchange rate, the current weather in Berlin, booking a doctor's appointment") |

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
| `Machine::form_context_extractor(language)` | `ExtractedFormValues`: one key per field of the form, each a string or null, like `HintMap` | Every value the utterance gives, several at once. A date as `YYYY-MM-DD`, a spoken date or time as the caller's words for the day or the time of day, in lowercase and in the caller's language when that is German, in English otherwise. Null for every key on a plain yes or no, a question or a request. A value changed only in part is written whole ("no, the tenth" → the full date) |
| `Machine::form_intent_matcher()` | `ExtractedFormIntent`: `machine_reasoning`, `caller_intent`, plus `confidence` | One intent of the form menu |
| `Machine::form_agreement_checker()` | `CheckedAgreement`: `is_question`, true or false | Only asked when the matcher chose `confirm_yes`, and only shown the utterance: does the caller ask something? A question is then matched as `get_information[form_information]` (`ExtractedFormIntent::as_question`), without a confidence, because the matcher's was in the agreement |
| `Machine::form_response_formulator(language)` | `FormulatedAnswer`: `spoken_response` | Only the answer to a question about the form, from `<form_state>`, in the language its first rule names. Its first sentence is kept (`into_spoken_answer`), and code asks the step's question after it |

The form menu, in the order the matcher sees it: `unsupported`, `provide_form_field_value`,
`correct_form_field_value`, `refer_to_context_for_form_field_value`, `confirm_yes`, `confirm_no`, `cancel_form`,
`get_information[form_information]`, `repeat`, `end_call`, `transfer_to_human`.

Weather, exchange rate, `calendar_help` and `start_form` are not in it. Asked inside a form they are `unsupported`: the
caller is told that this cannot be helped with right now and the pending question is asked again. With `calendar_help`
in the menu, "в следующую субботу" said as the appointment date was matched as a calendar question (confidence
0.9998), so the form could not be finished.

### What each intent does (`form_intent_context_handler`)

| Intent | The form | The caller hears first, from the vocabulary |
|---|---|---|
| `provide_form_field_value`, `refer_to_context_for_form_field_value` | Every value the utterance gives for a field that is not confirmed yet is parsed with its field's kind, shown to the field's validators and recorded. Each of those fields then waits for its own confirmation. Values for later fields alone accept the one read back for the current field | Nothing for a recorded value: the step's question reads the current one back. The validator's sentence for a value it refused. `not_understood` for a value that cannot be read and for an answer that gave none. Each sentence once |
| `correct_form_field_value` with a new value | The same for every field, a confirmed one too, which is opened again. It never accepts the value read back, whichever fields it is for | The same |
| `correct_form_field_value` without a new value | As `confirm_no` | As `confirm_no` |
| `confirm_yes`, once the agreement checker found no question in the utterance | The current field is confirmed. With another value for the field that was read back ("yes, but it's 1992"), that value replaces it and waits for confirmation. Whatever else the extractor wrote is not taken | `thanks`. On the last field the form's `completed` sentence and nothing else |
| `confirm_no` | The current field's value is dropped and its rejection counter goes up | `sorry`. From the third rejection of one field `offer_human` |
| `get_information[form_information]`, also a question the matcher took for a yes | Unchanged | The formulator's answer, its first sentence |
| `repeat` | Unchanged | The last reply once more, word for word. No step after it |
| `cancel_form` | Dropped, the call is back in the main menu | The form's `cancelled` sentence. No step |
| `end_call` | Dropped, the call is back in the main menu, action `end_call` | `goodbye`. No step |
| `transfer_to_human` | Kept, action `transfer_to_human` | `transfer`. No step |
| `unsupported` and anything else | Unchanged | `cannot_help_in_form` |

While the form is still open the reply ends with the question of its step (`next_step(form, language)`), except
after `repeat` and `transfer_to_human`: the field's `ask` sentence, or its `confirm` sentence with the value as it
is spoken (`FormFieldValue::spoken`: a date in words, a text without the full stop the caller's sentence ended
with). It is always about the current field, the first one that is not confirmed. A field that got its value in an
earlier answer is therefore not asked for when the call gets to it: the caller is only asked whether the value is
right.

```
caller  Hans Müller, geboren am 13. Juni 2091.
reply   Ein Geburtsdatum kann nicht in der Zukunft liegen. Ich habe als Namen des Patienten Hans Müller notiert.
        Ist das richtig?
caller  Ja
reply   Danke. Wie lautet das Geburtsdatum des Patienten?
```

A yes or no when nothing is waiting for confirmation changes nothing, and the pending question is asked again. The one
exception is a yes-or-no field that has no value yet: there the yes or no is taken as its value. The doctor form has no
such field.

### Decided in code, not by the model

1. **The extracted values only count when the intent fills a field.** The extractor writes values the form already
   knows, and ones from the history, on many "yes", "no" and question turns, so its values are carried next to the
   intent (`IntentMatched::Form(intent, values)`) and used only where the intent fills.
2. **A value the field already holds is not a value the caller gave** (`given_values`, `holds`). The extractor writes
   most of what the form holds once more in every answer. Without this a plain "нет" that the matcher took for a
   correction recorded the old name again.
3. **A correction without a value is a rejection.**
4. **Values for later fields alone confirm the current one** when that one was waiting for confirmation: the caller
   moved on from the value that was read back. Only for `provide` and `refer`, never for a correction, and not when
   the utterance also gives a value for the current field. It is decided once for the whole answer, before anything
   is filled, so a field filled by the same answer is never taken for the one read back.
5. **An agreement with another value is a correction** (`replaces_read_back`). It has to be sure, because the extractor
   also writes the held value once more in its own words: the field's kind can read the value, and for a text the new
   words were said in the utterance, for a date or a number the value differs. Never for a yes-or-no field. Only the
   value for the current field is taken then.
6. **Another value in place of the one that was read back says that one is wrong**, even when the new one cannot be
   read or a validator refuses it. The read-back value is dropped and the field is asked for again.
7. **A value a validator refuses is not recorded** (see Validators).
8. **A completed form is sent only after its reply was formulated**, so a turn that fails sends nothing.
9. **The extractor's keys are the fields an utterance can give a value for** (`Form::answerable_fields`, put into
   the JSON schema by `ExtractedFormValues::fit_schema`). That is every field, except a later field that takes a like
   kind of value as the one the caller is on (`FormFieldKind::is_like`), unless that one takes text. For the doctor
   form: while the date of birth is asked for or read back, `appointment_date` is not a key. With it, "Nein, am
   vierzehnten" said to a date of birth that was read back was filed under the appointment date in 3 of 3 replays,
   and "Irgendwann im Sommer" too. A day and a time said in one answer need no such rule any more: both have a key.
10. **An answer changes no value the caller has confirmed.** `provide` and `refer` fill fields that are not confirmed
    yet; a confirmed one takes a correction, which the matcher chose for "Moment, der Name ist falsch, …" in 11 of 11
    logged turns.
11. **Who words the reply.** The intent handler returns it (`Reply`). In a form it is code's on every turn but
    one: what the turn has to say first, then the step's question, each a sentence of the vocabulary. Left to the
    formulator, the reply read back every value the caller gave, skipped a confirmation or asked about a value
    again (see Verification, "Replies worded by code"). Only the answer to a question about the form is the
    formulator's, and of that the first sentence is kept: told to answer in one sentence and ask nothing, the model
    went on with a question or a step of its own in 16 to 25 of 49 replayed answers.
12. **A question is never an agreement.** What the matcher took for `confirm_yes` is shown to the agreement checker
    (`ContextExtractedHandler`), which sees the utterance and nothing of the form. When it finds a question, the
    turn is a question about the form: the value that was read back stays unconfirmed, the formulator answers, and
    the step's question is asked again. With the form in front of it the matcher takes a question for a yes when
    it asks whether something is right or confirmed: 535 of 1,344 questions it had never seen, and asked about the
    last value that completed the form and sent it. A machine that fails here fails the turn, like any other.

    ```
    reply   Ich habe als Namen des Patienten Hans Müller notiert. Ist das richtig?
    caller  Ist der Termin damit schon bestätigt?
    reply   Der Termin ist noch nicht bestätigt. Ich habe als Namen des Patienten Hans Müller notiert. Ist das richtig?
    ```

### When a turn fails

`CallSession` remembers the state the turn found (`state_before_turn`). `fail_turn` puts that state back, sets the
action to `continue` and saves a fixed apology as the reply, a phrase of the vocabulary:

- main menu (`turn_failed_in_main_menu`): the apology plus what the assistant offers;
- form (`turn_failed_in_form`): "Sorry, an error happened on our side, please say that again." The form is as it
  was, so the caller only has to say it again.

---

## Forms (`domain/form.rs`)

- `Form { kind, fields }`, `FormField { name, description, kind, value, state, confirmation_failed_counter }`.
  A field is a step: `Queued` → `AwaitingConfirmation` → `Completed`; the current field is the first not completed.
- Methods: `field` (builder: the name and the kind, the description is the vocabulary's), `current_field`,
  `find_field`, `is_filled`, `is_ahead`, `answerable_fields` (see "Decided in code", 9),
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
  (spoken_date), `appointment_time` (spoken_time). What each is described as, asked with and read back with is
  under `[forms.doctor_appointment.fields.<name>]` in the vocabulary. A spoken date and a spoken time are not alike
  for `is_like`, so the two can be neighbours.
- `FormFieldValue::spoken(language)` is the value as the caller hears it: a date in words ("13. Juni 1991",
  "June 13, 1991", "13 июня 1991 года", "13 червня 1991 року"), anything else as it is held, without the full stop
  a caller's sentence ended with.

### Validators

A validator is an async function that gets the app state and the value a field is about to take:

```rust
async fn date_of_birth_not_in_the_future(_: &AppState, value: &FormFieldValue) -> Result<(), ValidationError> {
    …
    Err(ValidationError::DateOfBirthInTheFuture)
}
```

- `ValidationError` is an enum with a variant per reason. What the caller is told is the variant's text in the
  vocabulary (`[validation.date_of_birth_in_the_future]`). The flow says it before the step's question, which asks
  for the field again.
- **Which field has which validators** is the `match (self, field)` in `FormSupported::validate`, under `build()`. To
  add one, write the function and add an arm, and for a new reason a variant and its text. Several for one field:
  `first(app_state, value).await?; second(app_state, value).await`.
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
  container's clock (`TZ` from `APP_TIMEZONE`), and `response_context` when a formulator is asked.
- `call_turn_outcome` (`CallTurnOutcome`) lasts for the turn too: `caller_intent`, `machine_reasoning`, `confidence`,
  `action`. It is what the HTTP response is built from.
- **Language:** `CallerSpokeHandler` sets the call's language from every utterance of three words or more that
  lingua can tell the language of (`MIN_WORDS` in `classifier.rs`: words of two letters or more, numbers are not
  words). A shorter one keeps the language the call has: a bare "13. Juni 1991" was read as English and "голова
  болит" as Ukrainian, and the reply is in the call's language word for word now. A new call starts in
  `call_settings.default_language` (`de`). `language` in the request is only logged.
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
  | `selected_function`, `reasoning`, `confidence` | `call_turn_outcome`, written by the intent handler of the flow the call is in. `null` when the turn failed before an intent was matched. For a question the form's matcher took for a yes, `selected_function` is `get_information[form_information]`, `confidence` is `null` and `reasoning` is still the matcher's, which can read like an agreement |
  | `language_detected` | the call's language as an ISO 639-1 code |
  | `action` | `continue`, `end_call` or `transfer_to_human` |
  | `response_context` | `call_turn_context["response_context"]`, what the intent handler gave the response formulator. `null` when no formulator was asked: on a turn code worded (the turns of a form but a question about it, and the turn that starts one), and when the turn failed before it |
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
- `cargo check -p app --all-targets` (31 warnings, see Leftovers). `cargo test -p app -- domain:: vocabulary::
  classifier::` runs the 23 tests that need no services (they read `config/llm_vocabulary.toml`, which cargo finds
  from `app/`); the five in `tests.rs` are about the config, Postgres and Redis.
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

**Several values per answer and the question about the last form, 2026-10-06.** `cargo test -p app domain::`: 11
pass. The 521 distinct form extractor prompts logged that day were replayed with one key per field in place of the
one `form_field` / `form_field_value` pair, and their answers set next to the logged ones:

- **A day with a time, and the caller's own first live answer.** "Morgen um 15 Uhr", "Tomorrow at 3 pm", "Nächsten
  Samstag vormittags" and "Next Saturday in the morning" filled both fields in every replay (13 of 13; before, the
  pair had the day only, or the time only). "Anthony Joshua so I'm experiencing a bit of a headache today I was
  hoping I can make it to you lunchtime tomorrow", which had recorded the name alone in the live call, gave the name,
  the reason, the day and the time. A live call with that sentence was completed in 8 turns and 16 before.
- **The examples of the format rule decide the language of a spoken day or time.** With the English examples for
  every caller, "Übermorgen um neun Uhr" came out as "overmorgen" or "next monday" in 6 of 6 replays (the pair had
  kept "übermorgen" in 5 of 6) and "halb elf" as "half eleven". In a live German call the form was completed with
  `appointment_date: next monday`: the reply read back the caller's words, not the value. Without examples the
  English answers were worked out ("Next Saturday" as `2026-10-14`, three wordings, one of them the hint
  extractor's). German and English examples in one rule, or a third English one, leaked into the other language's
  answers ("Tomorrow at 3 pm" as "übermorgen" in 7 of 7, as "the day after tomorrow" in 4 of 7). With German
  examples for a German call: "übermorgen" in 5 of 6 and once "nächsten montag", "halb elf" in 2 of 3 and once the
  example "15 uhr".
- **A day said on the date-of-birth step.** With `appointment_date` as a key there, "Nein, am vierzehnten" was
  filed under it in 3 of 3 and "Irgendwann im Sommer" too. Without the key and with the German examples: the date
  of birth is corrected in 1 of 3, and in 2 of 3 nothing is extracted, which the flow takes as the denial and asks
  for the date again.
- **Values the form holds are written again in most answers**, word for word when the examples and the held value
  are in one language, which `given_values` drops. Held in English and asked with German examples, the value came
  back translated ("nächsten samstag" for "next saturday") in 7 of 18 replays of a plain yes.
- **Live, over HTTP against the app container:** 20 calls (10 German, 10 English) brought to an answer that gives
  two to five values recorded every value. `evals/run.py -r 2` (330 test cases, 508 turns): Form State 282/290 in
  German and 115/122 in English, Intent 296/306 and 142/150, Completed Form 10/10 and 6/6, the date-and-time goldens
  8 of 8, the question about the last form matched in 6 of 6 turns in both states. With the two whole-call goldens
  that still scripted the time said again brought up to date, the goldens this change touches had Form State
  24/24 in German and 17/18 in English. The one miss is `name-moving-on-en`, which failed in the runs before too.
- **Still open:** on a turn that recorded several values the formulator tends to read all of them back and ask
  whether "that" is right, or to skip the confirmation and ask for the next empty field, and the caller's yes then
  confirms the current field only. Four wordings of the response context for such a turn made no difference over
  20 turns (9 to 13 replies of 20 flagged in each). "Er ist am 13. Juni 1991 geboren und hat Rückenschmerzen" said
  to a name that was read back was matched as `confirm_yes` in 2 of 2 runs, so both values were lost.

**The next step in a tag of its own, 2026-10-06** (commit `dc2293d`). The 438 distinct logged form formulator
prompts that had a next step were replayed in four layouts, and every reply checked with the suites' own `Reply
Asks`, `No Early Booking`, `Reply Language` and `Reply Speakable`:

| Layout | Flagged, de (of 314) | Flagged, en (of 124) |
|---|---|---|
| The step at the end of `<response_context>`, with the rules of before | 57 | 6 |
| `<next_step>` in front of `<response_context>`, as committed | 79 | 25 |
| The same with the rule "Follow `<response_context>` instructions…" kept as it was | 71 | 30 |
| The same without the abbreviation rule | 74 | 29 |
| `<next_step>` as the last tag of `<context>` | 68 | 21 |

- The usual miss is a reply that takes the value for confirmed and asks for the next field ("That's correct, June
  13th, 1991. Next, could you please tell me the reason for your visit?").
- The 67 prompts of a turn that completes the form were not affected (0 to 1 flagged in every layout).
- On the turn that starts a form the main menu formulator wrote the reply then, and it has no rule about
  `<next_step>`: 11 of 38 German replies flagged, against 6 as before and 6 with such a rule; English 6, 7 and 5
  of 19. The form formulator writes that reply now, see "Which formulator writes a form's first reply".
- On the 20 turns that recorded several values: German 5 of 10 flagged with the tag as committed, 1 of 10 with the
  tag last and 1 of 10 with the step inside `<response_context>`; English 6, 6 and 5 of 10.
- The rule "Follow `<response_context>` instructions when answering and take its content into consideration" was
  put back into the form formulator after that, on the user's word. Nothing else was changed for it.
- The suites' goldens that look for the step in `response_context` (`STARTED` in
  `test_main_menu.py`, `ASK_NAME` and the two confirmations in `test_hints.py`) fail on `Response Context` since
  then, 10 goldens outside the known gaps in both passes of the run above, and the judged metric gets the
  instruction without the step.

**Which value is confirmed first when an answer gives several, 2026-10-06.** 40 turns that put two to five values
into the form at once were played over HTTP, 20 on the utterance that starts the form (hints) and 20 inside it, half
German and half English. Their formulator prompts were replayed in other layouts. Counted: replies that ask to
confirm the current field, which is the first of the values (`Reply Asks`), of 20.

| The formulator's prompt | Start of a form | Inside the form |
|---|---|---|
| As built: the main menu formulator on the starting turn, the form formulator inside, `<next_step>` in front | 11 | 12 |
| The main menu formulator with the form formulator's rule about `<next_step>` | 17 | – |
| The form formulator on the starting turn too | 14 | 12 |
| The same, `<next_step>` as the last tag | 14 | 15 |
| The form formulator without `<form_state>` | 20 | 16 |
| The same, `<next_step>` as the last tag and "Confirm nothing else in this reply: the other values are confirmed later, one at a time." added to a confirmation step | 18 | 19 |

- **What the formulator sees matters more than which formulator it is.** With `<form_state>` in its prompt it reads
  back every value the form holds, or takes the one it should confirm for settled and asks for the next empty field
  ("Perfekt, Lukas Schneider wurde am 4. März 2015 geboren. Wann … würdest du für den Termin preferieren?"). Over
  the 57 logged turns that start a form: 17 flagged with the main menu formulator as built, 17 with the form
  formulator in its place, 4 with the form formulator and no `<form_state>`. Over the 438 logged ordinary steps:
  101 flagged with the tag in front and the state, 87 without the state; 93 and 85 with the tag last; 63 and 70 with
  the step at the end of `<response_context>`.
- **Tried and not kept as ideas:** no `<conversation_history>` either (it helped the 40 turns and broke ordinary
  steps: 183 flagged of 438, replies in the wrong language, ISO dates, statements in place of questions); a machine
  of its own that only words the step (13 and 14 of 20); the step's question as a second key of the formulator's
  answer (the first key went on asking its own question in 254 of 439 ordinary steps, 216 passed at best).
- What was suggested to the user from this: the form formulator writes the reply of every turn that leaves the
  call in a form, and it is shown `<form_state>` only for a question about the form. It was built the same day in
  a narrower form, see the next section.

**Which formulator writes a form's first reply, and when the formulator is shown the form, 2026-10-06.** Three
builds of the app container were measured the same way: the tree as it was, the suggestion as written (the form
formulator on the starting turn, `<form_state>` only for a question about the form), and what is built now (the
form formulator on the starting turn, `<form_state>` left out on a turn that put a value into the form and on the
starting turn). `cargo test -p app domain::`: 12 pass.

*Paired replay.* Every distinct logged formulator prompt of the old tree, answered again in each layout. Flagged
by `Reply Asks`, `No Early Booking`, `Reply Language` or `Reply Speakable`:

| 417 ordinary form steps | As it was | As suggested | Built |
|---|---|---|---|
| All, de (249) | 37 | 44 | 28 |
| All, en (168) | 49 | 45 | 35 |
| **All** | **86** | **89** | **63** |
| One value recorded, de / en (90 / 73) | 8 / 22 | 6 / 7 | 5 / 7 |
| Several values recorded, the first to be confirmed, de / en (37 / 30) | 6 / 15 | 4 / 16 | 4 / 16 |
| After a yes, next field asked for, de / en (61 / 44) | 5 / 6 | 17 / 14 | 4 / 6 |
| After a yes, next field has a value to confirm, de / en (21 / 11) | 2 / 3 | 4 / 5 | 2 / 3 |
| After a no, de / en (13 / 2) | 5 / 0 | 3 / 0 | 4 / 0 |
| A value refused or not readable, de / en (6 / 4) | 0 / 0 | 2 / 0 | 0 / 0 |

A second run of the same replay gave 84, 89 and 65.

| 52 turns that start a form | Flagged, de (of 30) | Flagged, en (of 22) |
|---|---|---|
| Main menu formulator with `<form_state>`, as it was | 12 | 6 |
| Main menu formulator without it | 10 | 0 |
| Form formulator with it | 13 | 6 |
| Form formulator without it, as built | 5 | 0 |

- **Without the form the reply to a yes asks about the confirmed value again** ("Hans Müller wurde am 13. Juni
  1991 geboren. Ist das richtig?" where the step was to ask for the reason). 16 of the 137 turns after a yes were
  flagged with the form, 39 without. Four other wordings of "The caller confirmed <field>. Thank them." without the
  form: 41, 33, 32 and 25. So the form stays in on a turn that put no value into it.
- **With the form the reply to a recorded value skips its confirmation** ("Perfekt, Hans Müller. Könnten Sie
  bitte Ihre Geburtsdatum angeben?"): 30 of 163 turns that recorded one value flagged with it, 13 without.
- **The turn that completes a form** is answered alike by both formulators (64 prompts: 0 flagged with the form
  formulator, 1 with the main menu one), so it stays with the form formulator.

*Live, over HTTP.* The 40 turns that put two to five values into the form at once (see the section above), three
times each:

| Of 60 | As it was | As suggested | Built |
|---|---|---|---|
| Start of a form: asks for the current step | 31 | 60 | 60 |
| … and says no other value | 3 | 18 | 18 |
| … flagged | 32 | 0 | 0 |
| Inside the form: asks for the current step | 42 | 54 | 55 |
| … and says no other value | 6 | 6 | 6 |
| … flagged | 21 | 12 | 12 |

`evals/run.py -r 2` (330 test cases, 504 turns), reply checks per turn:

| | As it was | As suggested | Built |
|---|---|---|---|
| Turns in a form flagged, de (of 230) | 34 | 22 | 13 |
| Turns in a form flagged, en (of 90) | 30 | 15 | 20 |
| Turns that start a form flagged, de / en (of 44 / 20) | 6 / 4 | 8 of 46 / 0 | 6 / 0 |
| Reply Asks in a form, de / en | 198/228, 77/90 | 213/228, 81/90 | 218/228, 78/90 |
| No Early Booking in a form, en | 75/90 | 86/90 | 84/90 |
| Goldens outside the known gaps, de / en | 156/202, 56/98 | 162/202, 70/98 | 165/202, 64/98 |
| Repeated Sentences, de / en | 9/18, 2/6 | 6/18, 4/6 | 7/18, 2/6 |

Intent, Form State, Hint Extraction, Completed Form and Call Action moved by 0 to 2 turns between the three runs.
In the run of the suggestion a restaurant booking was matched as the doctor form in both passes (confidence 0.55),
which is the two extra starting turns. The English goldens of one point of the form share one walk, so 15 against
20 is within what one run tells apart (four of the 20 are two goldens of one point whose replies hold an ISO
date); the replay above is the measure for that.

- **What did not change:** a reply to several values still tends to read all of them back before it asks about
  the first (18 of 60 at the start of a form and 6 of 60 inside say no other value). The question at the end is
  about the right field.
- **New with the form formulator on the starting turn:** when that turn confirms a prefilled name, the reply can
  hold the field's description word for word ("Can you confirm that Full name of the patient is Emma Clark?", and
  for one German utterance "…bestätigen, dass Full name of the patient ist Mia Wagner?"): 12 of 60 live starting
  turns, 3 of them German, 2 of 60 inside the form, and 2 of 64 starting turns in the suites. 0 before. Two
  rewordings of the rule "Say forms, fields and values in plain words…" changed nothing (replay of the 52 starting
  turns: 4 and 5 against 5). The step worded as a sentence ("…confirm that
  the full name of the patient is…") took them to 0 and raised early booking claims on turns that record a value
  from 15 to 27 of 233, so it was not kept.
- **Measured and not built** (233 turns that recorded a value, 52 starting turns, flagged): "Confirm nothing else
  in this reply: the other values are confirmed later, one at a time." added to a confirmation step when other
  values wait: 34 to 28 and 5 to 4, replies that say no other value 13 to 17 of 70 and 9 to 16 of 30.
  `<next_step>` as the last tag: 37 and 7, with eight replies that hold an ISO date.
- The starting turn's formulator call takes about 50 ms longer than before (median 251 to 307 ms): the form
  formulator has more rules. The other steps of a turn are as they were.

**Replies worded by code, and the vocabulary, 2026-10-06 (evening).** `task1.md` is the plan of it in plain words.
`cargo test -p app -- domain:: vocabulary:: classifier::`: 20 pass.

*Why code words the steps.* The 580 distinct formulator prompts the tree before it had logged (365 German, 215
English) were replayed in other shapes. Flagged: a reply that fails one of the suites' four reply checks, says a
value it was not asked to confirm, says a sentence of the reply before again, says a field's description word for
word, makes the weather up, speaks an intent's label, or asks whether it is right once the form is complete.

| Who words the reply | Flagged of 580 | de / en |
|---|---|---|
| The formulator, as it was | 167 | 106 / 61 |
| The formulator with the best prompt shape found: the step forced into its answer as a first key (a one-value `enum`), the instructions in the user message after the utterance, one turn of history, only the current value named on a turn that recorded several, facts in words | 71 | 61 / 10 |
| Code for the steps of a form, the formulator for the rest, from facts in words | 2 | 1 / 1 |
| Qwen3-8B-AWQ in place of Qwen2.5-7B, the prompts as they were | 218 | 145 / 73 |
| Qwen3-8B-AWQ, the best prompt shape / code for the steps | 59 / 2 | |

- One lever at a time: the forced first key 126, the instructions in the user message 145, one turn of history 155,
  the first two together 109, with one turn of history 78.
- Facts in words alone: completion replies that asked "is that right?" 11 of 12 German to 0, spoken intent labels
  5 of 8 to 0, made-up weather 10 of 29 to 2.
- **Tried and not kept:** a `pattern` that requires a final "?" (junk to `max_tokens` for a reply that was no
  question); `maxLength` (cuts the sentence off); "Thank them in one or two words and do not say the confirmed
  value again" (the reply drops the step: "Perfekt!", 150 to 172 flagged); dates as they are spoken inside
  `next_step` with the formulator still wording it (German replies skipped the confirmation and asked for an
  "Adresse", 0 to 9 of 104); an English draft the model translates ("Hat den Namen des Patienten Hans Müller? Ist das
  korrekt?", 78 of 365 German flagged); the history as the chat's own messages (121).
- **Bigger models do not fit the card.** Qwen3.5-9B AWQ takes 7.55 GiB and left no KV cache at 0.80 or 0.87 of the
  12 GB, and 0.90 is all the desktop leaves. Gemma 3 12B, Gemma 4 12B and Qwen3-14B are larger. Qwen3-8B-AWQ fits
  with `--max-model-len 8192` and thinking switched off (`chat_template_kwargs: {"enable_thinking": false}`).

*The answer to a question about the form.* The plan was a JSON schema `pattern` without a question mark. Replayed
on 49 logged questions, the model wrote the Arabic question mark in its place, or went on without one ("Ich habe
Hans Müller notiert. Ist das korrekt eingetragen, oder möchten Sie etwas ändern, bevor wir fortfahren können, Herr
Müller."), one answer ran to `max_tokens` and one request ended in a vLLM 500 (`grammar rejected tokens`). Without a
pattern 16 of the 49 answers held a question and 25 more than one sentence, and the first sentence was a fit
answer in 49 of 49. So code keeps the first sentence.

Then 140 questions were asked over HTTP, seven at each of ten points of the form in German and English:

- **97 were matched as a question about the form** (52 of 70 German, 45 of 70 English). The rest: 19 taken for a
  yes, 13 turned down as an outside request, 10 taken for an answer without a value, 1 for the end of the call.
- **The 19 are one question:** "Ist der Termin damit schon bestätigt?" (9 of 10) and "Is the appointment confirmed
  now?" (10 of 10). 10 of them were asked while a value was being read back, and there the yes confirms the value
  and the form goes on. The other 9 met nothing to confirm and changed nothing.
- **None of this is new.** The same 140 questions were put to the matcher with the history the tree before it had
  at those points (the formulator's own replies) and with the history it has now (code's sentences): 89 and 96
  matched as a question, the "confirmed" question taken for a yes in 19 of 20 both times, 12 and 11 turns that
  change the form or the call wrongly.
- Of the 103 distinct answers 41 had more than one sentence and 1 a first sentence that was a question.
- **"Haben Sie den Termin schon gebucht?" was answered in English** ("No, we haven't booked the appointment yet.")
  at most points of the form, 6 of 57 German answers in all. Replayed: the language named in the instruction 5,
  the instruction in the user message 7, the rule about being booked taken out 2, **the language named in the rule
  ("Reply in German, which is the language the caller speaks") 1**. That rule is what the form's formulator has.

*The prompts of the other five machines did not change.* Their role, their rules and the intent descriptions, as
the vocabulary gives them, were set next to the prompts the tree before it logged: equal, every one.

*The language of a short utterance.* 151 utterances, the suites' and the short answers a form gets: 9 tagged
wrongly when every utterance sets the language, 4 with three words or more. What is left is in gap 1.

*Suites.* `evals/run.py -r 2` (330 test cases, 504 turns), the tree before it (`report_20261006_174839.md`) and
what is built (`report_20261006_204323.md`):

| | Before, de | After, de | Before, en | After, en |
|---|---|---|---|---|
| Goldens outside the known gaps that pass every check | 165/202 (82%) | 196/208 (94%) | 64/98 (65%) | 92/100 (92%) |
| Language Detection | 346/350 | 350/350 | 148/154 | 150/154 |
| Intent | 300/304 | 294/304 | 144/148 | 144/148 |
| Form State | 286/288 | 282/288 | 120/120 | 120/120 |
| Reply Language | 346/350 | 350/350 | 148/154 | 150/154 |
| Reply Speakable | 350/350 | 350/350 | 150/154 | 154/154 |
| Reply Asks | 330/346 | 346/346 | 140/152 | 152/152 |
| No Early Booking | 274/274 | 276/276 | 104/110 | 110/110 |
| Repeated Sentences | 7/18 | 18/18 | 2/6 | 6/6 |
| A form turn as the caller waits, median / p95 | 1133 / 1740 ms | 759 / 1201 ms | | |
| A main menu turn, median / p95 | 1032 / 1438 ms | 828 / 1302 ms | | |

- **No turn fails first at its reply any more.** The ten goldens that fail do so at the language detector (the two
  English rate questions, detected as German), at an extractor ("Übermorgen" written as "nächsten montag", and a
  year said in words read as 1999 in one of the two runs) or at a matcher (another city, another currency and a
  restaurant booking taken for the supported request, and the question below).
- **Worse than before:** "Welche Angaben brauchen Sie von mir?", asked for the name, is matched as
  `refer_to_context_for_form_field_value` (confidence 0.36) in every run and was a question about the form before.
  The matcher's prompt is the same but for the history, which holds code's sentences now. The caller hears
  "Das habe ich leider nicht verstanden." and the question for the name again.
- The goldens of three gaps pass and lost their `known_gap`: a bare German date (gap 1), and the first reply of a
  form when the caller gave their own name, which asked for a date of birth or said the booking was done.
- The suites take what code says from the vocabulary, by its key (`phrase`, `refusal`, `form_says`, `ask`,
  `confirm` in `suite.py`), so a reworded sentence needs no change in a golden. `Repeated Sentences` does not
  count a sentence of the vocabulary. `Response Context` is only looked at on a turn a formulator words.

**A question must not count as a yes, 2026-10-06 (night).** `task2.md` is the problem as it was found: "Ist der
Termin damit schon bestätigt?", asked while a value was read back, was matched as `confirm_yes` and confirmed the
value. `cargo test -p app -- domain:: vocabulary:: classifier::`: 23 pass.

*How big it was.* A logged matcher prompt sent to vLLM again gives the logged answer: 140 of the 140 questions, and
1,832 of the 1,832 other distinct prompts of that day (1 differed between two replays).

- **Six walks through the form**, live (the suites' two and four with other patients, short answers and a form
  started from hints), the question asked at each of their 56 points: a yes in 53, the value confirmed in 28 of 30
  read-backs, and each of the 6 asked about the last value completed the form and handed it to the submitter.
- **It is a class of questions, not one.** 48 questions that are in no log and no suite, 24 German and 24 English,
  at those 56 points: 535 of 1,344 a yes, 325 of the 720 said to a value that was read back. "Haben Sie das richtig
  notiert?", "Stimmt denn alles, was Sie bisher haben?", "Did you get that right?" and "Is everything correct so
  far?" were a yes in 15 of 15 read-backs. Written without punctuation 398 of 1,008, and 478 of 1,344 in lowercase
  too.
- **It is not the label's name.** With `confirm_yes` and `confirm_no` renamed to `agree` and `deny` the question
  was a yes in 17 of 20, with the state `awaiting_confirmation` renamed to `read_back` in 13 of 20.

*What was tried*, each by replay:

| | The question, a yes (of 20, of 56 on the other walks) | Unseen questions, a yes (of 1,344) | Logged agreements kept (of 617) | Other logged answers changed (of 1,196) |
|---|---|---|---|---|
| The matcher as it was | 19, 53 | 535 | 617 | |
| A rule that a question is never a yes, the best of seven wordings | 2, 6 | 119 | | |
| "is it confirmed now" as an example in the description of the form question | 0, 1 | | | |
| The rule's first wording with such examples | 1, 5 | 139 | | |
| `confirm_yes` described as "never a question" | 20, 53 | | | |
| A key `is_question` in front of `caller_intent`, and code that never confirms on it | 0, 0 | 12 | 617 | 49 |
| The key behind `caller_intent` | 5, 7 | 51 | | |
| A machine of its own for what the matcher took for a yes, shown the call like every machine | 0, 0 | 9 | 617 | 0 |
| **The same, shown the utterance alone (built)** | **0, 0** | **0** | **617** | **0** |

- **A rule fits the question it was written for.** It left 119 to 139 of the unseen questions a yes, and it moved
  answers it is not about: "Is 3 pm possible?", said where a value was asked for, was an answer in 13 of 13 and a
  question about the form in 12 with the rule. So the matcher's prompt was left as it is.
- **The key moves the matcher's other answers.** Of the 49: "hmm" to a value that was read back became a denial in
  5 and a yes in 1, a plain "Nein, das stimmt nicht." a correction in 5. Of 150 unseen denials said to a read-back,
  59 became corrections. By my reading about 12 of the 49 are worse and 7 better.
- **The check is right when it does not see the form.** Shown the history and the context it missed 9 of the 535,
  8 of them "Can you confirm the appointment?", and took 280 ms. Shown the utterance alone it missed none and takes
  86 ms (median of 60 calls, 89 at p95). Of 2,003 agreements, logged and unseen, it took none for a question either
  way.
- **Wordings that backfired for the check:** `machine_reasoning` in front of its answer took 15 of the 2,003
  agreements for a question ("Ja", "Yes", "да"); "true when the caller only asks something, … also when a question
  comes after that" took 354.

*The check on 414 utterances of its own* (225 questions, 189 agreements; German, English, Russian, Ukrainian), each
as written, without punctuation, and in lowercase too:

| | As written | Without punctuation | And in lowercase |
|---|---|---|---|
| Questions seen as one, de | 93/94 | 92/94 | 91/94 |
| en | 88/89 | 88/89 | 88/89 |
| ru | 24/24 | 15/24 | 14/24 |
| uk | 18/18 | 11/18 | 10/18 |
| Agreements not seen as one | 189/189 | 188/189 | 188/189 |

- The German and English misses: "Wie bitte?" and "Pardon?" as written, and without punctuation "Passt das so für
  Sie", "Hören Sie mich", "ist damit alles erledigt", "ist bei ihnen alles korrekt angekommen", "wäre auch 15 uhr
  möglich".
- **A Russian or Ukrainian yes-or-no question without its question mark reads like a statement** ("запись уже
  подтверждена"), and the check takes it for one. A question with a question word is seen.
- A rule about speech recognition ("may have no question mark and no capital letters: go by its words and their
  order") found 634 of the 675 question forms in place of 632, so it was not added.
- An agreement with a question after it ("Ja, das stimmt. Wie geht es weiter?") is an agreement for the check in 5
  of 10 wordings, and an agreement that asks back ("Ja, stimmt, oder?") a question in 6 of 6.

*Built, and asked over HTTP against the app container:*

| | Before | After |
|---|---|---|
| The question of `task2.md`, a yes (of 20) | 19 | 0 |
| The 140 questions, matched as a question about the form | 97 | 116 |
| … that change the form or the call | 11 | 1 |
| The 1,344 unseen questions, a yes | 535 (replay) | 0 |
| … said to a value that was read back, which they confirm (of 720) | 325 (replay) | 0 |
| … matched as a question about the form | 407 (replay) | 935 |
| 52 unseen agreements at the 30 read-backs, value confirmed (of 780) | 757 (replay) | 756 |
| An agreement with a question after it, value confirmed (of 120) | 73 (replay) | 25 |

- The unseen utterances were asked on the build before the formulator's rule below got its word. The matcher and
  the check are the same in both builds.
- **The answers to the question:** "Der Termin ist noch nicht bestätigt." in 10 of 10 German turns, and all 10
  English answers say that it is not confirmed yet, in several wordings.
- The 24 agreements that did not confirm are "Fine", "Okay" and "Uh-huh", which the matcher takes for `unsupported`
  as before. The check took no agreement for a question.
- **What still changes the call on a question** is the matcher's own: 48 of the 1,344 were matched as `end_call`
  ("Are we done?" in 19 of 28, "Do you need anything else?" in 23 of 28), and 3 said to a read-back as `confirm_no`.
  See gap 5.
- **The form formulator's rule** says "never say it is booked, confirmed or done" now. Its 1,162 prompts of these
  runs, answered again: an answer that says the appointment is booked or confirmed in 22 before and 13 with the
  word, and among the 282 questions with "bestätig" or "confirm" in them in 17 and 5. "Ist der Termin jetzt
  bestätigt", without its question mark, was told that the appointment is confirmed in 4 answers before ("Der
  Termin ist jetzt bestätigt.") and in none after.

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
- Makes facts up from raw data: weather code 3 (overcast) was "sonnig", "Nebel" and "the sun is out", a wind from
  287 degrees came "aus Südwesten", and intent labels were said as "UAH pro EUR abrufen".
- Words a step on its own terms when it is told what to do and not what to say: reads back every value the caller
  gave, takes a value for confirmed, says the same sentence again after a yes.
- Takes a question for an agreement when it asks about what the form is waiting for ("Ist das damit bestätigt?",
  "Haben Sie das richtig notiert?"): 40% of unseen questions with the form in its prompt, none with the utterance
  alone. A rule against it fits the wording it was written for, and a rule or a key added to a matcher's prompt
  moves answers it is not about (49 of 1,196 logged ones). A narrow second question in a prompt of its own moves
  nothing.
- Does not stop at a JSON schema `pattern` that forbids what it wants to write. With "?" forbidden it wrote the
  Arabic question mark, or went on to `max_tokens`, or vLLM ended the request with a 500 (`grammar rejected
  tokens`). A pattern that requires a final "?" has it write junk for a reply that was no question.

---

## Known gaps and next steps

**Language**
1. **The call's language follows every utterance of three words or more, and the detector is wrong on some of
   those.** "What is the euro to hryvnia exchange rate?" and "How many hryvnias do I get for one euro?" are detected
   as German as a call's first utterance (4 of 4 turns in the suites). Russian and Ukrainian are confused in
   sentences of three to five words ("Так, усе правильно." as Russian, "Болит голова уже три дня." as Ukrainian),
   and in a form the whole reply is then in the other language, since code says it in the call's language. A call
   that starts with fewer than three words stays in the default language until a longer utterance comes. Ways out:
   `with_minimum_relative_distance` on the lingua builder in `classifier.rs`, so an unsure detection keeps the
   call's language, or the language the phone side sends with the request.
2. **Chinese inside Russian replies** of a formulator (8 of 143 Russian form replies on 2026-10-05). A form's
   replies are code's now, so what is left of it is the main menu and the answer to a question about the form. A
   `pattern` on `spoken_response` in the JSON schema, allowing Latin, Cyrillic, digits and punctuation, turned all 8
   into correct Russian on replay. Not applied. `updates.md` has the pattern.
3. **The Russian and Ukrainian sentences of the vocabulary** were written without a native speaker and have been
   heard in two short calls only. They avoid a verb that says whether the assistant is a man or a woman ("Имя
   пациента — {value}. Всё верно?").
4. Some Russian names come out transliterated in hints.

**Dialogue**
5. **A question about the form** is matched as one in about 8 of 10 (116 of 140 asked live). It no longer counts
   as a yes (see "Decided in code", 12). What is left is the matcher's, counted on 1,344 unseen questions asked
   live:
   - **A question taken for the end of the call** drops the form and ends the call: 48 of them, "Are we done?" in
     19 of 28 and "Do you need anything else?" in 23 of 28, and "What else do you need from me?" in 1 of 10. The
     agreement checker sees these as questions too. Not built: whether a question may end a call ("Kann ich jetzt
     auflegen?") is not decided.
   - A question taken for a denial drops the value that was read back: 3 of 720 ("Steht der Termin jetzt?").
   - A value asked as a question ("Wäre 15 Uhr möglich?") while another value is read back is an answer for a
     later field, and like every such answer it accepts the value that was read back (46 of 120).
   - Turned down or not understood: "Wie viele Fragen kommen noch?" and "Warum fragen Sie das?" as `unsupported`,
     "Welche Angaben brauchen Sie noch von mir?" as an answer without a value in half of its turns.
   - **A Russian or Ukrainian yes-or-no question without its question mark** reads like a statement, and the
     checker takes it for one (9 of 24 and 7 of 18 missed), so it can still count as a yes. In German 2 to 3 of 94
     are missed without punctuation ("ist damit alles erledigt"), in English 1 of 89.
   - **An agreement with a question after it** ("Ja. Und jetzt?") is taken for the question in about half of its
     wordings, and an agreement that asks back ("Ja, stimmt, oder?") always: the value stays unconfirmed, the
     question is answered and the caller is asked again (25 of 120 confirmed, 73 before).
   - The answer is the formulator's: 13 of 1,162 still say the appointment is booked or confirmed ("Ich habe den
     Termin für morgen bestätigt." to "Können Sie mir den Termin bestätigen?"), and an answer can promise what the
     form does not know ("… erhalten Sie eine Bestätigung per E-Mail").

   When a question is about the value being read back, the caller hears it twice ("Ich habe den Namen Hans Müller
   notiert. Ich habe als Namen des Patienten Hans Müller notiert. Ist das richtig?"). A repeated reply starts with
   the "Danke." of the turn before. The main menu's formulator still works from an instruction: "Hi, how are
   you?" got no list of what is offered in 1 of 2 runs, and a joke was told where the refusal was due.
6. **A value matched as `confirm_yes` while its field has no value is lost.** After "no, it is 2092" was refused and
   the read-back value dropped, "June 13th 1991" was matched as `confirm_yes` in 1 of 6 English calls. Nothing was
   recorded, the reply read the date back anyway, and the caller's "yes" had nothing to confirm.
7. `repeat` in the main menu tells the formulator to repeat its reply "from `<conversation_history>`", which arrives
   escaped; the form flow quotes the reply instead. Not changed in the main menu. Unclear words ("хм", "так", "ну",
   "hmm") are taken for `confirm_yes` in 7 to 9 of 160 matcher calls.
8. "Andrew with a v" is recorded with the words in it.
9. On a question about the completed form the context extractor fills the hints from `<conversation_history>`
   (3 of 3 calls: name, date of birth, appointment). Since the completion callback the hints hold the form's values
   anyway. `get_information[form_information]` and `get_information[last_filled_out_form_information]` are two
   intents: the first is only in the form menu, the second only in the main menu.
10. Several values per answer, what is still open (see Verification):
    - Values for two later fields said to a value that was read back were matched as `confirm_yes`, and what the
      extractor wrote is not taken on an agreement, so they were lost and are asked for.
    - An answer the asked field cannot take can be recorded for a later field it fits ("Irgendwann im Sommer" as
      the time of the appointment in 1 of 3 replays). It waits there to be confirmed.
    - Only a German caller's days and times are kept in their language. A Russian or Ukrainian one's are written
      in English, with the wrong translations that showed for German. Not measured.
    - When the language detected for an utterance is not the one a held day or time is written in, the extractor
      writes the held value again, translated. For the field that is read back that counts as another value: it
      is read back once more and a value for a later field no longer accepts it.
    No final confirmation of the whole form; no "end the call anyway?" inside a form; no `required` flag; no skip
    or don't know; no `greeting`, `who_are_you` intents.
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
    Postgres and Redis. A form that an older build saved in Redis with a field the vocabulary no longer has panics
    when its question is asked, until the session's hour is over.

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
