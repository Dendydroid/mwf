# Updates — 2026-10-05

What changed in the working tree on top of commit `3ea4edf` ("fail errors 1"): the form flow, the event handlers,
the HTTP endpoint and the cleanup that followed, with what was tested and what the tests showed.

Nothing is committed. The two file renames and the deletion are staged (they were done with `git mv` and `git rm`),
everything else is unstaged.

---

## In short

- **A call can fill the doctor appointment form.** It asks for each field, has every value confirmed, and is back
  in the main menu once the form is completed, cancelled or the call ends. A completed form goes to `FormSubmitter`.
- **`POST /assistant/handle-request` works again**, on the same events as `--prompt-loop`, with the same six
  response keys as before.
- **The old path is gone**: `event/caller_intent.rs`, the old machines in `machine.rs`, the commented-out handler in
  `api.rs`. Compiler warnings went from 126 to 31, and none of the 31 comes from the old path.
- **Events are one file per event**, and the structs they carry moved out of the event files into `domain/flow.rs`.
- **Checked** with `cargo check --all-targets`, the form unit test, scripted conversations through the prompt loop
  against the local vLLM and Redis (23 of them after the form menu got its final shape), and calls over HTTP. With
  the final prompts English was right in every turn. Russian holds its state and intents, but its replies have
  problems that are listed below and were not fixed.

---

## What a turn does now

```
CallSessionLoadedEvent   InitialContextHandler     puts current_time into the turn's context

CallerSpokeEvent         CallerSpokeHandler        1. detects the language of the utterance
                                                   2. extracts context
                                                        main menu  ->  HintMap
                                                        form       ->  ExtractedFormValue (one field and its value)

ContextExtractedEvent    ContextExtractedHandler   matches the intent
                                                        main menu  ->  merges the hints, ExtractedMainMenuIntent
                                                        form       ->  ExtractedFormIntent

IntentMatchedEvent       IntentMatchedHandler      changes the call's state and formulates the reply
                                                        main menu  ->  main_menu_intent_context_handler,
                                                                       main_menu_response_formulator
                                                        form       ->  form_intent_context_handler,
                                                                       form_response_formulator

FormCompletedEvent       FormSubmitter             POSTs the completed form to FORM_SUBMIT_URL
```

Which side runs is decided by `CallState`: `Idle` is the main menu, `FormInProgress(form)` is the form. Every turn
is three model calls in both menus. A form turn took about one second on the local GPU.

---

## Where things are

| File | Holds |
|---|---|
| `app/src/domain/flow.rs` | At the top what both menus share: `FlowContext`, `IntentMatched`, `Reasoning`, `FormulatedResponse`. Then `main_menu_flow` (with `HintMap`) and `form_flow`. |
| `app/src/domain/call_session.rs` | `CallSession` with the new `save_last_exchange`, `fail_turn` and `CallTurnOutcome` |
| `app/src/event/call_session_loaded.rs` | `CallSessionLoadedEvent`, `InitialContextHandler` |
| `app/src/event/caller_spoke.rs` | `CallerSpokeEvent`, `CallerSpokeHandler` |
| `app/src/event/context_extracted.rs` | `ContextExtractedEvent`, `ContextExtractedHandler` |
| `app/src/event/intent_matched.rs` | `IntentMatchedEvent`, `IntentMatchedHandler` |
| `app/src/event/form_completed.rs` | `FormCompletedEvent`, `FormSubmitter` |
| `app/src/event/events.rs` | registration of all five |
| `app/src/routes/api.rs` | `handle_assistant_request` on the event system |
| `app/src/settings.rs` | the only `is_prompt_loop_mode` and `PROMPT_LOOP_FLAG` |

An event file now holds its event and its handler and nothing else. `event/mod.rs` lists them in the order a turn
dispatches them.

### Moved and renamed

| Before | Now |
|---|---|
| `HintMap` in `event/context_extracted.rs` | `main_menu_flow` in `domain/flow.rs` |
| `FlowContext` in `event/context_extracted.rs` | top of `domain/flow.rs` |
| `IntentMatched` in `event/intent_matched.rs` | top of `domain/flow.rs` |
| `SpokenResponse`, `FormulatedResponse` in `main_menu_flow` | top of `domain/flow.rs`, used by both formulators |
| `fail_turn`, `save_last_exchange`, free functions in `event/intent_matched.rs` | methods on `CallSession` |
| `event/call_session.rs`, `CallSessionLoaded` | `event/call_session_loaded.rs`, `CallSessionLoadedEvent` |
| `event/form.rs`, `FormCompleted` | `event/form_completed.rs`, `FormCompletedEvent` |
| `is_prompt_loop_mode` and its const in both `main.rs` and `settings.rs` | `settings.rs` only, imported by `main.rs` |

`is_prompt_loop_mode` stays in `settings.rs` because `AppSettings::load` needs it before anything else exists.

### Removed

- `app/src/event/caller_intent.rs` (`IntentExtracted`, `CallStateHandler`). Its form logic is ported to `form_flow`.
- From `domain/machine.rs`: `Machine::intent_matcher`, `response_formulator`, `calendar_refuser`, `form_informant`
  and the second copy of `SpokenResponse` / `FormulatedResponse`.
- From `domain/call.rs`: `unsupported_backend_context`, `is_offered`, `all_labels`.
- From `routes/api.rs`: the stub, the commented-out old handler and its three constants.
- From `event/events.rs`: the commented-out registration of the old handler.
- From `ExtractedMainMenuIntent`: the unused `new` and `intent()`.
- Unused imports in every file listed above and in `main.rs`, `call.rs`, `machine.rs`, `settings.rs`.

---

## The form flow (`form_flow` in `domain/flow.rs`)

### Machines

| Machine | Output | Job |
|---|---|---|
| `Machine::form_context_extractor()` | `ExtractedFormValue`: `form_field` and `form_field_value`, both optional | The one field and value the utterance gives. A date as `YYYY-MM-DD`, a spoken date as the caller's words. Nothing for a plain yes or no, a question or a request. |
| `Machine::form_intent_matcher()` | `ExtractedFormIntent`: `machine_reasoning`, `caller_intent`, plus `confidence` read from the token probabilities | One intent of the form menu |
| `Machine::form_response_formulator()` | `FormulatedResponse`: `spoken_response` | The sentence the caller hears, from `<response_context>` |

The form menu, in the order the matcher sees it: `unsupported`, `provide_form_field_value`,
`correct_form_field_value`, `refer_to_context_for_form_field_value`, `confirm_yes`, `confirm_no`, `cancel_form`,
`get_information[form_information]`, `repeat`, `end_call`, `transfer_to_human`. The descriptions are the ones in
`Described for CallerIntent` in `call.rs`, taken the same way the main menu takes them.

Weather, exchange rate, `calendar_help` and `start_form` are not in the form menu. Asked inside a form they are
`unsupported`: the caller is told that this cannot be helped with right now and the pending question is asked
again. `calendar_help` was in the menu at first and was taken out because of what the runs showed: "в следующую
субботу", said as the appointment date right after a calendar question, was matched as a calendar question with a
confidence of 0.9998, so the form could not be finished.

### What each intent does (`form_intent_context_handler`)

| Intent | The form | The formulator is told |
|---|---|---|
| `provide_form_field_value`, `refer_to_context_for_form_field_value`, `correct_form_field_value` with a new value | The value is parsed with the field's kind and recorded for the field the extractor named, or for the current one. The field then waits for confirmation. | `Recorded "…" for <field>.`, or that the value could not be understood, or that no value was given |
| `correct_form_field_value` without a new value | As `confirm_no` | As `confirm_no` |
| `confirm_yes` | The current field is confirmed | `The caller confirmed <field>. Thank them.` On the last field: that the form is complete, with all values, and to ask whether the caller needs anything else |
| `confirm_no` | The current field's value is dropped and its rejection counter goes up | That the value was wrong and dropped, and to apologize. From the third rejection of one field: to apologize and offer a human |
| `get_information[form_information]` | Unchanged | To answer in one sentence and say the next step in a sentence of its own |
| `repeat` | Unchanged | To say the last reply once more, with the reply quoted |
| `cancel_form` | Dropped, the call is back in the main menu | That the form was cancelled, and to ask whether the caller needs anything else |
| `end_call` | Dropped, the call is back in the main menu, `action` is `end_call` | A short goodbye |
| `transfer_to_human` | Kept, `action` is `transfer_to_human` | To say the caller is being transferred |
| `unsupported` and anything else | Unchanged | That this cannot be helped with while the form is being filled in, and not to answer it |

While the form is still open, the next step is added at the end of the response context, except after `repeat`
and `transfer_to_human`: `Next, ask the caller for: <description>.` or
`Next, ask the caller to confirm that <description> is <value>.`

A yes or no when nothing is waiting for confirmation changes nothing, and the pending question is asked again. The
one exception is a yes-or-no field that has no value yet: there the yes or no is taken as its value, as in the old
code. The doctor form has no such field.

### Decided in code, not by the model

1. **The extracted value only counts when the intent fills a field.** The extractor writes a value the form already
   knows on many "yes", "no" and question turns, so its value is carried next to the intent
   (`IntentMatched::Form(intent, value)`) and used only for the three intents that fill.
2. **A value the field already holds is not a value the caller gave** (`holds`). Without this a plain "нет" that
   the matcher took for a correction recorded the old name again.
3. **A correction without a value is a rejection.**
4. **A value for a later field confirms the current one** when that one was waiting for confirmation: the caller
   moved on from the value that was read back. Ported from the old code.
5. **A completed form is sent only after its reply was formulated**, so a turn that fails sends nothing.

The main menu still fills `HintMap`. The form does not read it yet.

### When a turn fails

`CallSession` remembers the state the turn found (`state_before_turn`). `fail_turn` puts that state back, sets the
action to `continue` and saves a fixed apology as the reply:

- main menu: the text that was there already, which lists what the assistant offers;
- form: "Sorry, an error happened on our side, please say that again." in German, Russian, Ukrainian or English.
  The form is as it was, so the caller only has to say it again.

In the main menu this is what happened before (the state was set to `Idle`). In a form it means one failed model
call no longer loses the values given so far.

---

## HTTP endpoint (`routes/api.rs`)

`handle_assistant_request` checks `request_text`, dispatches `CallerSpokeEvent` with the session the middleware
loaded, and builds the response from the session. Loading, the per-call lock and saving stay in
`call_session_middleware`.

```json
{
  "answer": "The full name of the patient is John Smith. Is this correct?",
  "selected_function": "provide_form_field_value",
  "confidence": 1.0,
  "language_detected": "en",
  "action": "continue",
  "reasoning": "The caller provided the value for the patient's name."
}
```

| Key | From |
|---|---|
| `answer` | `data.last_spoken_response` |
| `selected_function`, `reasoning`, `confidence` | `call_turn_outcome`, written by the intent handler of the menu the call is in. `null` when the turn failed before an intent was matched |
| `language_detected` | the call's language as an ISO 639-1 code |
| `action` | `call_turn_outcome.action`: `continue`, `end_call` or `transfer_to_human` |

`call_turn_outcome` (`CallTurnOutcome` in `call_session.rs`) lasts for the turn only, like `call_turn_context`.

**A failed turn is now a 200 with the apology as `answer`**, as in the prompt loop, not a 502 or 504.
`ApiError::Inference` is therefore never produced. It is still in `error.rs`, untouched, in case the endpoint
should answer with an error status again.

---

## Changes inside code that was already there

- `ExtractedMainMenuIntent` has `confidence` again, read in `read_answer` like the old intent struct did, because
  `AssistantResponse` still has that key.
- `main_menu_intent_context_handler` writes the turn's outcome, and sets the action for `end_call` and
  `transfer_to_human`. The two TODO comments are still there.
- `FormSubmitter` is registered again in `events.rs`. It was commented out together with the old handler.
- `domain/information.rs`: the `FormInformation` text now says to answer in one sentence and to say the next step in
  a sentence of its own. The comment above it no longer points at machines that are gone.
- `domain/form.rs`: two comments say "context extractor" where they said "intent matcher".

---

## How it was checked

### Build and unit test

- `cargo check --all-targets`: no errors, 31 warnings (126 before).
- `cargo test -p app every_form_builds`: passes. The other five tests need Postgres and Redis and were not run.
- No tests were added. The form steps (`fill`, `confirm`, `reject`) are covered by the live runs only.

### Live conversations

Scripted conversations through `--prompt-loop` against the local vLLM (`Qwen/Qwen2.5-7B-Instruct-AWQ`) and Redis.
Four scripts: an English form from start to completion with a question about the form in the middle; an English one
with an unclear word, a denial, a correction of an earlier field, a calendar question, repeat, cancel and transfer;
and two Russian ones on the same path, one in short colloquial answers and one in fuller sentences.

"Right" means the utterance got the intent it should, the form was in the state it should be in afterwards, and the
reply asked what that state expects.

| | Conversations | Turns | Result |
|---|---|---|---|
| English, final prompts | 3 | 40 | Right in every turn |
| Russian, final prompts | 2 | 32 | State and intents right. Of 29 form replies, 3 had Chinese in them and 3 came in Ukrainian. One of the 3 was cut off at `max_tokens` and became a failed turn; after it that conversation was out of step with its script, and one answer in that part was matched as `unsupported`. |
| Everything after the form menu was narrowed | 23 | 340 | 289 form turns with a known expected intent: 7 wrong, all in Russian. 290 form replies: 8 with Chinese in them, all in Russian. |

The seven wrong intents: a plain "нет" taken for a correction (3), "в следующую субботу" taken for `unsupported`
(2), "да" taken for `unsupported` (1), a reason for the visit taken for a question about the form (1). Rules 2 and
3 above are there to turn a "нет" that is taken for a correction into a rejection. They cannot do it when the
extractor also makes up a new value for it, which happened once ("голова не болит").

The wording of a few rules still changed between these runs, so the last row is a picture of the flow as a whole,
not a measurement of one prompt. Three earlier conversations, run while `calendar_help` was still in the form menu,
are what led to taking it out; they are not in the table.

### Choosing the wording of rules

One run says little: vLLM does not always give the same answer at temperature 0 when two answers are close, and one
different reply changes the history of every turn after it. Wordings were compared by replaying every logged prompt
of one kind with the change applied.

- **Form matcher**, 139 logged calls. The rules as they are in the code: 4 wrong. Three variants that spell out which
  state leads to which intent: 5 to 7 wrong, because they turned the unclear word "here" into `confirm_no`. A replay
  of single calls had pointed the other way.
- **Form formulator**, 140 logged calls, checked for another script in the reply and for a confirmation step whose
  reply asks nothing or does not read the value back. Four wordings of the confirmation rule: 13, 9, 10 and 10
  replies flagged. The one with 9 is in the code: "say what the value is for, read it back and ask plainly whether
  it is right".
- **Tried and not kept**: "a date the way it is spoken" in the formulator (14 flagged against 13, and a live reply
  that spelled the 13th out as "шестнадцатое"); a rule against working out dates (the Russian conversation run
  after it had Chinese in 2 of its 16 turns, both about the date of birth).

Smaller things the runs led to:

- `repeat` quotes the last reply.
- After a "no" the formulator is told to apologize. Before, it repeated its old confirmation question.
- After a "yes" it is told to thank. Before, it said "The preferred date of the appointment is queued".
- The reply to something unsupported says not to answer it. Before, it made up a date for "what date is next
  saturday?".

### Failure paths

- **Formulator fails after the form was completed** (it happened by itself, see problem 1): the caller heard the
  apology, the form was back with its last value still waiting for confirmation, nothing was submitted.
- **Any machine fails inside a form, over HTTP**: a second server with an unreachable vLLM address answered one turn
  of a running call: 200, the apology, `selected_function` `null`, `action` `continue`. The next "yes" on the healthy
  server confirmed the name, so the form had survived.
- **A machine fails in the main menu**: the Russian apology that lists what is offered.
- **Bad requests**: no `x-call-id` and an empty `request_text` both give 400 with `{"error": …}`.
- **Actions**: "goodbye" inside a form gave `action: end_call`, "connect me to a human" gave `transfer_to_human`.

The scripts and logs of these runs were throwaway and are not in the repo.

---

## Problems found and not fixed

They are outside what was asked, so they are reported here and left alone.

### 1. Chinese inside Russian replies

In 3 of 29 Russian form replies with the final prompts, and 8 of 143 over all runs, the formulator switched to
Chinese in the middle of the sentence. It happened when asking for the appointment date, when reading the date of
birth back, and in the sentence that says the form is done. Six of the 8 were valid JSON and would be spoken as they
are. Two ran on to `max_tokens`, which cuts the JSON off; those are failed turns now and the caller hears the
apology.

A fix was tried on all 8 by replaying their prompts with a `pattern` on `spoken_response` in the JSON schema that
vLLM holds the answer to, allowing Latin, Cyrillic, digits and punctuation:

```
^[\u0020-\u007e\u00a0-\u024f\u0400-\u04ff\u2010-\u2027\u2030-\u203a]+$
```

All 8 came back as correct Russian. One of them:

```
before  Дата рождения患者，您说的是俄语吗？请确认一下您的日期OfBirth是1999年6月13日吗？
after   Дата рождения указана как тринадцатое июня девяносто девятого. Это правильно?
```

In the code this is a pattern hook on `ValueSchema`, one line in `json_schema()` in `domain/machine.rs`, and the
pattern on `SpokenResponse`. It would cover the main menu formulator too.

### 2. Short Russian answers are detected as Ukrainian

`CallerSpokeHandler` sets the call's language from every utterance. lingua took "нет, Иван Петренко", "голова болит"
and "болит голова уже три дня" for Ukrainian, every time that conversation was run. The replies to those turns
then came in Ukrainian in most runs. For the first of the three, the reply was wrong or had a Polish word in it
("ім'яpacjent") in 5 of the 8 runs, and that text stays in the history for the turns after.

A form is answered in short phrases, which is where a detector is least sure. Two ways out: give the lingua builder
in `classifier.rs` a `with_minimum_relative_distance`, so an unsure detection returns `None` and the call keeps its
language; or only switch the language on longer utterances.

### 3. The main menu hint extractor writes words that are not values

`HintMap` got `patient_full_name: "string"` from "Здорово, башка болит, надо к врачу" and four fields set to the
word `"null"` from "What is the weather in Berlin right now?". After a form it copied the name and the date of birth
from the history on "goodbye". The likely cause is the description `either non-empty string or null type.` together
with the type shown in `<json_output_format>`. Nothing reads `HintMap` yet, so nothing shows it to the caller.

### 4. Smaller ones

- **`repeat` in the main menu** tells the formulator to repeat its last reply "from `<conversation_history>`". Values
  in `<context>` are escaped, so the model reads `&lt;conversation_history&gt;`. When the form flow used that
  sentence, the model repeated an older reply, not the last one. The form flow quotes the reply now; the main menu
  was not changed and was not tested for this.
- **Confirmation questions in Russian** sometimes leave the value out ("Полное имя пациента верно?"). Whether the
  formulator does better without `<form_state>` in its prompt is not settled: it fixed 2 of 4 bad replies when those
  four were replayed, but over 140 replayed calls it had 16 replies flagged against 13.
- **Confidence does not point at these mistakes.** The two wrong intents measured before the menu was narrowed had a
  confidence of 0.9989 and 0.9998.
- **The prompt loop does not stop at the end of piped input.** It keeps reading empty lines, so a piped script has
  to end with `exit`.

---

## Not done

- From `refactor_plan.md` (`git show a9338c0:refactor_plan.md`): filling a new form from `HintMap` and writing the
  form's values back into it, the final confirmation of the whole form, asking "end the call anyway?" inside a
  form, several values in one utterance, the `greeting`, `who_are_you` and `this_call` intents.
- `context.md` still describes the design from before the refactor.
- The 31 warnings that are left are untouched: the cookie-session middleware and `session.rs`, the `db` and cache
  helpers, four `AppState` fields nothing reads (`db`, `session`, `llm`, `http_client`), the unused parts of the
  event bus, `llm_system_prompt_file` and `session_ttl_days` in the settings, `VllmClient::model` and `query_json`,
  four `AllowedValue` variants, the `call_id` fields of `CallerSpokeEvent` and `CallSessionLoadedEvent`, and unused
  imports in files that were not otherwise touched (`app.rs`, `db.rs`, `routes.rs`, `routes/middleware.rs`,
  `session.rs`).

---

## Running it

```bash
# prompt loop, from the repo root (reads .env, then .env.stdin, and app/config/settings.toml)
cargo run -- --prompt-loop

# the HTTP server against the local services: the localhost values are in .env.stdin
cd app && set -a && . ../.env.stdin && set +a && APP_PORT=8089 cargo run

curl -s -X POST http://localhost:8089/assistant/handle-request -H 'Content-Type: application/json' \
  -H 'x-call-id: test-call-1' -d '{"request_text": "I would like to book a doctor appointment"}'
```

In the log, `Machine prompt` and `Machine answered` show every model call. A finished form logs `Completed FORM`,
then `Form submitted`, or the warning that `FORM_SUBMIT_URL` is not set.
