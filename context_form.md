# Context — the form flow: several values per answer, the next step, which value is confirmed first

**Written:** 2026-10-06 · **Repo:** `H:\rust\ai_assistant`, branch `master`, last commit `dc2293d next_step added`.
Everything described under "Built" is in the working tree and **not committed**. The app container runs that tree.
**Model:** `Qwen/Qwen2.5-7B-Instruct-AWQ` on the local vLLM. Every number here is from that model.

`context.md` says what the code is as a whole. This file is about one day's work on the form flow: what was built,
what was measured about it, what was suggested and what became of it, and what is still open.

---

## In short

- **Built:** one answer can give several form values. Each is recorded and waits for its own yes, so a field that
  already has a value is only confirmed when the form gets to it, not asked for. Also built: a main menu question
  about the form that was filled out last.
- **Restored:** the rule "Follow `<response_context>` instructions when answering and take its content into
  consideration" in the form formulator. `<next_step>` stays a tag of its own, as committed in `dc2293d`.
- **Was wrong:** when one sentence gives two or more values, the reply should ask to confirm the first of them and
  nothing else. It asked about the first in 11 of 20 turns that start a form and 12 of 20 inside one.
- **Why:** the formulator was shown the whole `<form_state>`. It reads every value back, or takes the first one for
  settled and asks for the next empty field. Which formulator writes the reply matters less.
- **Suggested:** the form formulator writes every reply of a turn that leaves the call in a form, and it is shown
  `<form_state>` only for a question about the form.
- **Built from it, later the same day:** the form formulator writes the reply of the turn that starts a form, and
  `<form_state>` is left out of the formulator's prompt on a turn that put a value into the form and on the
  starting turn. Not on the other turns: without the form, the reply to a yes asks about the confirmed value again
  (finding 7).
- **Now:** the reply asks about the first value in 60 of 60 live starting turns (31 before) and 55 of 60 inside the
  form (42 before). It still says the other values first in most of them.
- **Open decision:** whether "Confirm nothing else in this reply…" goes into the step (a small gain, finding 7),
  and whether `<next_step>` stays in front of `<response_context>`.

---

## Built

| What | Where |
|---|---|
| The form extractor answers with one key per form field, each a string or null, like `HintMap` | `ExtractedFormValues` in `app/src/domain/flow.rs` |
| The keys come from the form in progress and `<json_output_format>` shows the same keys | `ExtractedFormValues::fit_schema`, and `json_output_format(schema)` in `app/src/domain/machine.rs` |
| Which fields are keys | `Form::answerable_fields` in `app/src/domain/form.rs` |
| A value its field already holds is not a value the caller gave | `given_values`, `holds` |
| Every given value is recorded in one go, and values for later fields accept the one read back, decided once per answer | `validate_and_fill`, `fill` |
| An answer (`provide`, `refer`) fills only fields that are not confirmed. A confirmed one takes a correction | `form_intent_context_handler` |
| German examples in the extractor's rule for a German caller, English ones otherwise | `Machine::form_context_extractor(language)` |
| `get_information[last_filled_out_form_information]`, main menu only. Summarizes `last_filled_out_form` if there is one, otherwise says that none was filled out | `GetInformationSupported::LastFilledOutFormInformation`, `HintMap::last_filled_out_form_information` |
| The turn that starts a form is written by the form's formulator | `IntentMatchedHandler` in `app/src/event/intent_matched.rs` |
| `<form_state>` is left out of the formulator's prompt on a turn that put a value into the form, and on the starting turn | `CallSession::shows_form_state` and `context()`, set in `form_intent_context_handler` and `start_form`; `Form::holds_new_value` |

The next step is always about the current field, the first one that is not confirmed:
`Next, ask the caller for: <description>.` or `Next, ask the caller to confirm that <description> is <value>.`
The code has this right in every case below. What goes wrong is the sentence the formulator makes of it.

`cargo test -p app domain::`: 12 pass. Goldens for the new behaviour are in `evals/test_form.py`,
`evals/test_conversations.py` and `evals/test_main_menu.py`.

---

## How it was measured

- **Replay.** The app logs every `Machine prompt` and `Machine answered`. A logged prompt is sent to vLLM again with
  one thing changed (the rules, a line of `<context>`, the JSON schema) and the answers are counted. The model does
  not always give the same answer at temperature 0 and one changed reply changes the history of every later turn,
  so a single live run does not rank two wordings. Many replayed prompts do.
- **Flagged** means a reply failed one of the suites' own checks (`evals/metrics.py`): `Reply Asks` (one question,
  and it is the form's next step), `No Early Booking`, `Reply Language`, `Reply Speakable`. They go by words, so a
  count is a direction, not an exact rate.
- **The sets:**
  - 521 distinct form extractor prompts logged on 2026-10-06, mostly from the eval runs.
  - 438 distinct form formulator prompts of an ordinary step (one value, or a yes or no), 314 German and 124 English.
  - 57 distinct prompts of a turn that starts a form, 38 German and 19 English, 26 of them with hints.
  - 40 turns played over HTTP that put two to five values into the form at once: 20 on the utterance that starts
    the form, 20 inside it, half German and half English.

---

## Findings

### 1. One key per field picks up what a single pair lost

Before, the extractor wrote one `form_field` and one `form_field_value`. Replayed with one key per field:

- "Morgen um 15 Uhr", "Tomorrow at 3 pm", "Nächsten Samstag vormittags", "Next Saturday in the morning": both the
  day and the time in 13 of 13. The pair had the day only, or filed everything under the time.
- "Anthony Joshua so I'm experiencing a bit of a headache today I was hoping I can make it to you lunchtime
  tomorrow" had recorded the name alone in a live call. Now: name, reason, day and time. That call took 16 turns
  before and 8 after.
- 20 live calls with two to five values in one answer recorded every value.
- The model writes most of what the form already holds into the keys again, word for word. `given_values` drops
  those. No reworded copy showed in the 521 replays as long as examples and held values were in one language.

### 2. The examples in the format rule decide the language of a day or a time

The rule for `spoken_date` and `spoken_time` ends in examples ("next saturday", "tomorrow", "in the morning",
"3 pm"). What the examples are decides what comes out:

| Examples | German answers | English answers |
|---|---|---|
| English, for every caller | "Übermorgen um neun Uhr" as "overmorgen" or "next monday" in 6 of 6, "halb elf" as "half eleven" in 3 of 3 | right |
| None (three wordings, one copied from the hint extractor) | as said | worked out: "Next Saturday" as `2026-10-14` in 4 of 4 |
| German and English in one rule | as said | the German example leaked: "Tomorrow at 3 pm" as "übermorgen" in 7 of 7 |
| A third English example ("the day after tomorrow") | "übermorgen" right in 6 of 6 | the example leaked: "Tomorrow at 3 pm" as "the day after tomorrow" in 4 of 7 |
| **German for a German caller, English otherwise (built)** | "übermorgen" in 5 of 6, once "nächsten montag". "halb elf" in 2 of 3, once the example "15 uhr" | right |

A wrong translation is not noticed: in a live German call the form was completed with `appointment_date: next
monday`, because the reply read back the caller's own word "übermorgen" and not the value.

Russian and Ukrainian callers still get the English examples. Not measured.

### 3. A day said on the date-of-birth step went to the appointment

With `appointment_date` as a key while the date of birth is asked for or read back, "Nein, am vierzehnten" was
filed under the appointment date in 3 of 3, and "Irgendwann im Sommer" too. `answerable_fields` leaves out a later
field that takes a like kind of value as the current one (`FormFieldKind::is_like`), unless the current one takes
text. For the doctor form that is exactly this one case.

Without the key, "Nein, am vierzehnten" corrects the date of birth in 1 of 3 replays. In the other 2 nothing is
extracted, the flow takes the correction for a denial, drops the date and asks for it again.

### 4. `<next_step>` as a tag of its own is followed less

Commit `dc2293d` moved the step out of the end of `<response_context>` into its own tag, which sorts in front of
`<response_context>`. Flagged replies over the 438 ordinary steps:

| Where the step is | With `<form_state>` | Without `<form_state>` |
|---|---|---|
| At the end of `<response_context>`, as before the commit | 63 | 70 |
| `<next_step>` in front of `<response_context>` (what runs now) | 101 | 87 |
| `<next_step>` as the last tag of `<context>` | 93 | 85 |

- The usual miss takes the value for confirmed and asks for the next field: "That's correct, June 13th, 1991. Next,
  could you please tell me the reason for your visit?"
- The rule wording is not the cause. With "Take `<response_context>` information into consideration…" in place of
  "Follow…" the first column reads 104 and 89, and without the abbreviation rule 103.
- A turn that completes the form is not affected: 0 to 4 flagged of 66 in every layout.

### 5. Which value is confirmed first when a sentence gives several

The 40 turns, replayed. Counted: replies that ask to confirm the current field, of 20. In brackets: of those, the
replies that also say none of the other values.

| The formulator's prompt | Start of a form | Inside the form |
|---|---|---|
| **As it runs now:** main menu formulator on the starting turn, form formulator inside, tag in front | 11 (1) | 12 (3) |
| Main menu formulator plus the form formulator's rule about `<next_step>` | 17 (6) | – |
| Form formulator on the starting turn too | 14 (2) | 12 (3) |
| The same, tag last | 14 (3) | 15 (8) |
| The same, tag last, and "Confirm nothing else in this reply: the other values are confirmed later, one at a time." added to the step | 19 (5) | 15 (8) |
| **Form formulator without `<form_state>`**, tag in front | 20 (6) | 16 (4) |
| The same, with the "Confirm nothing else…" sentence | 20 (10) | 16 (7) |
| Form formulator without `<form_state>`, tag last, with the sentence | 18 (8) | 19 (11) |
| Step at the end of `<response_context>`, without `<form_state>`, with the sentence | 16 (11) | 19 (11) |

What it says:

- **The guess that the main menu formulator on the starting turn is the cause holds only in part.** Putting the
  form formulator there moves 11 to 14. Over the 57 logged starting turns it moves nothing: 17 flagged with the main
  menu formulator, 17 with the form formulator.
- **`<form_state>` in the prompt is the bigger cause.** The same 57 turns with the form formulator and no
  `<form_state>`: 4 flagged. The 40 turns: 20 and 16 of 20. It also helps ordinary steps (table in 4).
- **The reply still tends to read the other values back** before it asks about the first one. The bracketed
  numbers stay around half at best. The question at the end is about the right field, so the caller's yes confirms
  what was asked.
- Typical replies as it runs now:
  - "Perfekt, Lukas Schneider wurde am 4. März 2015 geboren. Wann und wann Uhrzeit würdest du für den Termin
    preferieren?" (two values given, neither confirmed, a third field asked for)
  - "Great, we have your appointment scheduled for tomorrow at 3 pm. Is everything correct?"
  - "Great, John Smith. Now, could you please tell me your date of birth?" (one value, confirmation skipped)

### 6. Tried and not kept

| Idea | Result |
|---|---|
| No `<conversation_history>` for the formulator either | Good on the 40 turns (19 of 20 inside), bad on ordinary steps: 183 flagged of 438. Replies in the wrong language, ISO dates, statements in place of questions. The model uses its own earlier replies as its pattern |
| A machine of its own that only words the step, shown only the language, the response context and the step | 13 and 14 of 20 |
| The step's question as a second key of the formulator's answer, the reply being the two joined | The first key went on asking a question of its own in 254 of 439 ordinary steps. 216 of 439 passed at best |
| Another wording of `<response_context>` on a turn that recorded several values (only the current one, the others named, "do not read them back now") | 9 to 13 flagged of 20 in each of four wordings |
| Restoring the "Follow…" rule alone | 10 to 12 of 20 inside the form |

### 7. The suggestion built, and where the form has to stay in the prompt

Three builds of the app container, measured the same way: the tree as it was, the suggestion as written (items 1
and 2 below), and what is built now. Flagged replies unless said otherwise. The full tables are in `context.md`
(Verification, "Which formulator writes a form's first reply").

| | As it was | As suggested | Built |
|---|---|---|---|
| Replay, 417 ordinary form steps | 86 | 89 | 63 |
| … of them 137 turns after a yes | 16 | 39 | 16 |
| … of them 163 turns that recorded one value | 30 | 13 | 12 |
| Replay, 52 turns that start a form | 18 | 5 | 5 |
| Live, 60 starting turns with several values: ask about the first (say no other value) | 31 (3) | 60 (18) | 60 (18) |
| Live, 60 turns with several values inside the form: the same | 42 (6) | 54 (6) | 55 (6) |
| Suites, turns in a form, de (of 230) | 34 | 22 | 13 |
| Suites, turns in a form, en (of 90) | 30 | 15 | 20 |
| Suites, goldens outside the known gaps that pass, de / en | 156/202, 56/98 | 162/202, 70/98 | 165/202, 64/98 |

- **Without the form, the reply to a yes asks about the confirmed value again**: "Hans Müller wurde am 13. Juni
  1991 geboren. Ist das richtig?" where the step was to ask for the reason. After a yes the form is what tells
  the model that the value is settled. Four other wordings of "The caller confirmed <field>. Thank them." without
  the form: 25 to 41 flagged of 137, against 16 with the form.
- **With the form, the reply to a recorded value skips its confirmation.** So the form is left out exactly on the
  turns that put a value into it, and on the starting turn, where both formulators do better without it (main
  menu formulator 18 to 10, form formulator 19 to 5).
- **The turn that completes a form** is answered alike by both formulators (64 prompts, 0 and 1 flagged). It stays
  with the form formulator, so the only turn that changed hands is the one that starts a form.
- **New:** on a starting turn that confirms a prefilled name, the reply can hold the field's description word
  for word, "Can you confirm that Full name of the patient is Emma Clark?", and for one German utterance in a
  German reply. 12 of 60 live starting turns, 3 of them German; 0 before. Two rewordings of the plain-words rule
  did not change it. The step worded as a sentence did, and raised early booking claims on turns that record a
  value from 15 to 27 of 233.
- **Not changed by any of this:** the reply still says the other values before it asks about the first one.
- **Item 3 below, measured on top of what is built** (233 turns that recorded a value and the 52 starting turns):
  the "Confirm nothing else…" sentence 34 to 28 and 5 to 4 flagged, replies that say no other value 13 to 17 of
  70 and 9 to 16 of 30. `<next_step>` as the last tag 37 and 7, with eight replies that hold an ISO date.

---

## The suggestion

Items 1 and 2 were built in the narrower form of finding 7. Item 3 is not built.

1. **One formulator per form turn.** `IntentMatchedHandler` picks the formulator by the state the call is in once
   the intent handler ran. A turn that ends in `CallState::FormInProgress`, the one that starts the form too, is
   written by `Machine::form_response_formulator()`. The main menu formulator then never has to know about steps or
   confirmations.
2. **`<form_state>` for the formulator only when the caller asks about the form.** The extractor and the matcher
   keep it. For the formulator the intent handler says whether it is shown, the way it writes `response_context`:
   on for `get_information[form_information]`, off for every other intent and for `start_form`.
   `CallSession::context()` is where `form_state` is added today.
3. **Optional:** render `<next_step>` as the last tag, and add the "Confirm nothing else…" sentence to a confirmation
   step when other values wait. Inside the form that is 16 to 19 of 20. At the start of a form it reads 20 to 18,
   which is within what 20 turns can tell apart.

1 and 2 together are the row "Form formulator without `<form_state>`" above: 11 to 20 at the start of a form, 12 to
16 inside, 17 to 4 flagged over the 57 starting turns, 101 to 87 flagged over the 438 ordinary steps.

The step at the end of `<response_context>` is still the best layout for ordinary steps (63 flagged). That would
undo the tag of `dc2293d`, which was kept on purpose.

---

## Still open

- **The other values are still read back** on a turn that gave several (finding 7). The "Confirm nothing else…"
  sentence helps a little and is not built. The description said word for word on a starting turn is new.
- **Values lost on an agreement.** "Er ist am 13. Juni 1991 geboren und hat Rückenschmerzen", said to a name that
  was read back, was matched as `confirm_yes` in 3 of 3 eval runs. What the extractor wrote is not taken on an
  agreement, so both values were lost and are asked for. The golden `several-values-moving-on-de` is marked as a
  known gap.
- **An answer the asked field cannot take** can be recorded for a later field it fits: "Irgendwann im Sommer" as the
  time of the appointment in 1 of 3 replays. It waits there to be confirmed. Golden `birth-unreadable-de`, known gap.
- **A held value written again in another language.** When the language detected for an utterance is not the one a
  held day or time is written in, the extractor writes the held value again, translated ("nächsten samstag" for
  "next saturday" in 7 of 18 replays of a plain yes). For a confirmed field that is ignored. For the field that is
  read back it counts as another value: it is read back once more, and a value for a later field no longer accepts
  it. Short German answers are often detected as English (`context.md`, gap 1), so this can happen in a German call.
- **Goldens that look for the step in `response_context`** fail since `dc2293d`: `STARTED` in
  `evals/test_main_menu.py`, `ASK_NAME` and the two confirmations in `evals/test_hints.py`, 12 goldens. The HTTP
  response has no key for `next_step`, so the suites and the mimic client cannot see which step the formulator was
  given.
- **The last run of every suite** (`evals/run.py -r 2`, on what is built now, `results/report_20261006_174839.md`):
  Form State 286/288 in German and 120/120 in English, Intent 300/304 and 144/148, Completed Form 10/10 and 6/6,
  Reply Asks in a form 218/228 and 78/90. The run on the tree as it was is `report_20261006_164614.md`, the one on
  the suggestion as written `report_20261006_170839.md`.
