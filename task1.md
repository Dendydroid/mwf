# Task 1 — code writes the form's replies, the model only the free ones

**Written:** 2026-10-06 · **Status:** built the same day, not committed. The app container runs it.
`context.md` says what the code is as a whole. This file is the plan in plain words, and what came of it.

## Why

Before this, the response formulator (the third model call of a turn) wrote every reply. It got English notes from
the Rust code and had to make the sentence out of them. That is where most wrong replies came from: it read back
every value instead of the one to confirm, skipped a confirmation, said the same sentence twice, or said a booking
was done too early.

Measured by replaying the 580 distinct formulator prompts the old build logged (365 German, 215 English) and
counting the replies that fail the reply checks. The checks go by words, so the counts show a direction, not an
exact rate.

| Who writes the reply | Flagged of 580 | de / en |
|---|---|---|
| The model, as it was | 167 | 106 / 61 |
| The model, with the best prompt shape found | 71 | 61 / 10 |
| Code writes the form steps, the model only the free answers (this task) | 2 | 1 / 1 |

A newer model (Qwen3-8B) with the old prompts was worse (218), so the model is not what held the replies back.

## The idea in one line

Only the last step of a turn changed. The first two machines are exactly as they were. What changed is who writes
the sentence the caller hears.

---

## One turn, step by step

| Step | Before | Now |
|---|---|---|
| 1. Request comes in | `request_text` plus `x-call-id` | Same |
| 2. Language | Every utterance set the call's language | An utterance under three words keeps the language the call already has |
| 3. Context extractor | Takes the values out of the utterance | Same, no change |
| 4. Intent matcher | Picks one intent | Same, no change |
| 5. Intent handler (Rust code) | Changed the form. Then wrote two English notes for the model: `response_context` ("Recorded … for patient_name.") and `next_step` ("Next, ask the caller to confirm that …") | Changes the form the same way. Then builds the finished reply itself, in the caller's language |
| 6. Response formulator | Called on every turn. Read the notes and wrote the sentence | Called only when the reply needs free words. On most form turns it is not called at all |

After step 5 there are two paths. The handler says which one, as a `Reply`:

- **`Reply::Said(text)`, code has the whole reply** (most form turns): `IntentMatchedHandler` skips the formulator
  and passes the finished sentence to `session.save_last_exchange(...)`.
- **`Reply::Formulated { then }`, the reply needs free words**: the formulator is called, and code says `then`
  after it (the step's question, when a form is open).

---

## Where every text is

All of it is in one file: **`app/config/llm_vocabulary.toml`**. The code has no text of its own. It finds a text by
a key, and the key is an enum's variant or a form field's name.

| In the file | What it is | How the code gets it |
|---|---|---|
| `[phrases.<name>]` | A sentence code says in any form | `Phrase::Thanks.say(language)` |
| `[validation.<name>]` | What a validator tells the caller | `ValidationError::DateOfBirthInTheFuture.say(language)` |
| `[forms.<form>]` | What a form says when it is `started`, `completed`, `cancelled` | `form.kind.vocabulary().started.say(language)` |
| `[forms.<form>.fields.<field>]` | A field's `description`, its `ask` question and its `confirm` question | `next_step(form, language)` gives the question of the current step |
| `[dates]` | How a date is said: the format and the month names | `value.spoken(language)` |
| `[intents.<label>]` | An intent's `description` for the matcher, and how it is named to the caller (`offer`) | `intent.description()` |
| `[machines.<name>]` | A machine's `role` and `rules` | `Machine::form_intent_matcher()` and the others |
| `[output_values]`, `[prompt]`, `[instructions]`, `[facts]` | The other texts of a prompt | `vocabulary().instructions.end_call` and the like |

Three things to know about it:

- **`vocabulary()` is a global function.** It reads the file once, the first time it is asked. Everything above
  goes through it.
- **A text for the caller has four wordings:** `de`, `en`, `ru`, `uk`. All four are needed. A text for a machine is
  one English string.
- **A missing text stops the app when it starts**, not in the middle of a call. A key the file has and the code
  does not know stops it too.

A change to the file needs a restart of the app. In Docker that is `./update-app.sh`, because the file is copied
into the image.

### How to add something

| To add | In the code | In the file |
|---|---|---|
| A field to a form | One line in `FormSupported::build()` | `[forms.<form>.fields.<name>]` with `description`, `ask`, `confirm` |
| A sentence code says | A variant of `Phrase` | `[phrases.<name>]` |
| A validator's message | A variant of `ValidationError` | `[validation.<name>]` |
| Another wording of a sentence | Nothing | Change the text |

---

## Everything code says

Each sentence exists once per language.

**Per field**

- The question that asks for it (`ask`).
- The question that reads its value back (`confirm`, with `{value}`).

**Per validator**

- Its message to the caller.

**Per form**

- `started`: "Gern, vereinbaren wir einen Arzttermin."
- `completed`: "Vielen Dank, damit habe ich alles. Ich habe Ihre Terminanfrage weitergegeben. Kann ich sonst noch
  etwas für Sie tun?"
- `cancelled`: "In Ordnung, ich habe die Terminanfrage abgebrochen, es wurde nichts gespeichert. Kann ich sonst noch
  etwas für Sie tun?"

**In any form** (`Phrase`)

| Name | When | de |
|---|---|---|
| `thanks` | After a yes | Danke. |
| `sorry` | After a no | Entschuldigung. |
| `offer_human` | Third no for one field | Entschuldigung, das war wieder falsch. Wenn Sie möchten, verbinde ich Sie mit einer Kollegin oder einem Kollegen. |
| `not_understood` | Value not understood, or none given | Das habe ich leider nicht verstanden. |
| `cannot_help_in_form` | Outside request inside the form | Dabei kann ich im Moment leider nicht helfen. |
| `goodbye` | The caller ends the call inside a form | Vielen Dank für Ihren Anruf. Auf Wiederhören. |
| `transfer` | The caller asks for a human inside a form | Ich verbinde Sie jetzt mit einer Kollegin oder einem Kollegen. |
| `turn_failed_in_main_menu`, `turn_failed_in_form` | A machine failed | The two apologies that were in the code before |

**The date helper:** only dates need one. "13. Juni 1991", "June 13, 1991", "13 июня 1991 года", "13 червня 1991
року": a format and twelve month names per language.

Two cases need no text:

- **Repeat:** code says the last reply again.
- **A yes or no with nothing to confirm:** code just asks the step's question again.

For the doctor form that is 23 sentences per language: 10 for the fields, 1 for the validator, 3 for the form and
9 phrases.

---

## Who writes each kind of reply

| What happened in the turn | Before | Now |
|---|---|---|
| Form started | Model | Code: "Gern, vereinbaren wir einen Arzttermin." + step |
| One or more values recorded | Model | Code: the confirm question for the current field |
| Caller said yes | Model | Code: "Danke." + step |
| Caller said no | Model | Code: "Entschuldigung." + step |
| Third no for one field | Model | Code: the offer of a human + step |
| Validator refused the value | Model | Code: the validator's sentence + step |
| Value not understood, or none given | Model | Code: "Das habe ich leider nicht verstanden." + step |
| Outside request inside the form | Model | Code: "Dabei kann ich im Moment leider nicht helfen." + step |
| Last field confirmed | Model | Code: the form's `completed` sentence |
| Repeat | Model | Code: says the last reply again |
| Cancel, end call, transfer | Model | Code: one fixed sentence each |
| Question about the form ("Welchen Namen haben Sie notiert?") | Model | **Model writes the answer only**, code adds the step question |
| Main menu: weather, rate, last form, unsupported | Model | **Model**, as before |

So inside a form the model writes a reply in only one case.

---

## What the formulator still does

**Question about the form.** It gets `<form_state>` and the utterance, and is told to answer in one sentence and
ask nothing. Code then adds the step's question behind its answer. Its rules:

| Rule of the form formulator before | Now |
|---|---|
| Reply in the caller's language | Stays, but the language is named: "Reply in German, which is the language the caller speaks" |
| No markdown, lists, emojis | Stays |
| Follow `<response_context>` | Stays |
| Abbreviation rule | Stays |
| Plain words, no snake_case names | Stays |
| Answer only from `<form_state>` | Stays |
| Never say it is booked before the form is complete | Stays (for "did you book it?") |
| Only ask one question | Became "ask nothing", because code adds the step question |
| When `<next_step>` is set, end the reply with that step… | Gone, with the `<next_step>` tag |

Two things came out differently from the plan:

- **Code keeps only the first sentence of the answer.** The plan was a JSON schema that forbids a question mark.
  That did not work: the model wrote an Arabic question mark instead, or kept talking, and one request ended in a
  server error. Without it, a third of the answers went on with a question of their own after a good first
  sentence. So the first sentence is kept and the rest is dropped.
- **The language is named in the rule.** Told to reply "in the language given by `<language>`", the model answered
  the German "Haben Sie den Termin schon gebucht?" in English, in 6 of 57 German answers. With the language's name
  in the rule it was 1 of 57.

**Main menu.** Its role and rules did not change. It gets facts in words instead of raw data:

- "overcast, 20 degrees Celsius, wind of 6 kilometres per hour from the west-northwest" instead of the weather
  JSON. Before, it read weather code 3 (overcast) as "sonnig".
- "the euro to hryvnia exchange rate, the current weather in Berlin, booking a doctor's appointment" instead of
  the intent labels. Before, it said "UAH pro EUR abrufen" and "Formulare starten".

---

## One example

The caller says: "Hans Müller, geboren am 13. Juni 1991."

- **Before:** the handler wrote `Recorded "Hans Müller" for patient_name. Recorded "1991-06-13" for date_of_birth.`
  and `Next, ask the caller to confirm that Full name of the patient is Hans Müller.` The model read that and
  said: "Perfekt, Hans Müller wurde am 13. Juni 1991 geboren. Möchten Sie diesen Wert bestätigen?"
- **Now:** the handler records both values the same way. The current field is the name, so the reply is its
  `confirm` sentence: "Ich habe als Namen des Patienten Hans Müller notiert. Ist das richtig?" No model call.

After the yes, the reply is "Danke. Ich habe als Geburtsdatum den 13. Juni 1991 notiert. Ist das richtig?"

---

## What the suites say

`evals/run.py -r 2`, 330 test cases with 504 turns, before and after.

| | Before, de | After, de | Before, en | After, en |
|---|---|---|---|---|
| Goldens that pass every check | 165/202 (82%) | 196/208 (94%) | 64/98 (65%) | 92/100 (92%) |
| The reply asks the right step | 330/346 | 346/346 | 140/152 | 152/152 |
| No booking claimed too early | 274/274 | 276/276 | 104/110 | 110/110 |
| No sentence of the reply before said again | 7/18 | 18/18 | 2/6 | 6/6 |
| The reply is in the caller's language | 346/350 | 350/350 | 148/154 | 150/154 |
| A form turn as the caller waits, median | 1133 ms | 759 ms | | |

No turn fails first at its reply any more. The ten goldens that still fail do so earlier in the turn: the language
detector (two English questions about the rate are detected as German), an extractor ("Übermorgen" written as
"nächsten montag"), or a matcher (another city, another currency, a restaurant booking).

---

## What went away, and what it costs

**Went away for form turns**

- `response_context` and the `<next_step>` tag.
- The `shows_form_state` switching: the form is shown to the formulator only for a question about the form.
- Most rules of the form formulator.
- One model call on most form turns.

**Was updated with it**

- The suites: a golden names what code says by its key in the vocabulary, so rewording a sentence needs no change
  there.
- `context.md` and `evals/README.md`.
- The `response_context` key of the HTTP response is `null` on a turn code worded.

**Costs**

- 23 short sentences per language for the doctor form.
- A new field needs two sentences per language, and its description.
- Replies sound the same every time.

---

## Still open

- **The Russian and Ukrainian sentences** were written without a native speaker. They should be read by one.
- **A question about the form is not always taken for one.** Of 140 asked at every point of the form, 97 were.
  The worst case is a question with "bestätigt" or "confirmed" in it ("Ist der Termin damit schon bestätigt?"): it
  is taken for a yes almost every time. When a value is being read back at that moment, the value counts as
  confirmed although the caller did not agree. This is the intent matcher, not the reply, and it was like this
  before: with the old build's history the same questions came out the same (89 of 140 taken for a question,
  96 with the history as it is now).
- **"Welche Angaben brauchen Sie von mir?"** was taken for a question before and is taken for an answer without a
  value now. The matcher's prompt is the same, but the history in it holds code's sentences now.
- **Russian and Ukrainian are mixed up** in sentences of three to five words, and the whole reply is then in the
  other language.
- **A question about the value being read back** makes the caller hear it twice: "Ich habe den Namen Hans Müller
  notiert. Ich habe als Namen des Patienten Hans Müller notiert. Ist das richtig?"
- **The main menu's formulator** still works from an instruction, so it can still go wrong the old way.

---

## Tried and not worth doing instead

All on Qwen2.5-7B, same 580 prompts unless said otherwise.

| Idea | Result |
|---|---|
| A schema pattern that makes the reply end in "?" | The model cannot close the sentence and writes junk up to the token limit |
| A schema pattern that forbids "?" in the answer to a question about the form | An Arabic question mark instead, endless sentences, one server error (49 questions) |
| "Thank them in one or two words and do not say the confirmed value again" | The reply drops the step ("Perfekt!") |
| Dates written as spoken inside `next_step`, the model still writing the sentence | German replies skip the confirmation and ask for an "Adresse" |
| Code writes an English draft, the model translates it | "Hat den Namen des Patienten Hans Müller? Ist das korrekt?" |
| The language named in the instruction instead of in the rule | The German answer still came in English in 5 of 57 |
| Bigger models (Qwen3.5-9B, Gemma 3 12B, Gemma 4 12B, Qwen3-14B) | Do not fit the 12 GB card next to the desktop |
