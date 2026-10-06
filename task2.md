# Task 2 — a question must not count as a yes

**Written:** 2026-10-06 · **Status:** proposal. Nothing of it is built, and no fix has been tried yet.
The problem below was measured. The fixes are ideas, with a plan to measure them.

## The problem

Inside a form the intent matcher (the second machine of a turn) sometimes takes a caller's question for a "yes".
When the assistant has just read a value back, that "yes" confirms the value. The caller never agreed.

```
assistant  Ich habe als Namen des Patienten Hans Müller notiert. Ist das richtig?
caller     Ist der Termin damit schon bestätigt?
assistant  Danke. Wie lautet das Geburtsdatum des Patienten?
```

Three things went wrong in that turn:

- The name is marked as confirmed, although the caller did not say it is right.
- The caller's question was not answered.
- The form moved on, so the caller has no reason to notice.

The matcher even saw that it was a question. This is what it wrote as its reasoning, and then it chose
`confirm_yes`:

> The caller is asking if the appointment is confirmed now, which corresponds to the current state of awaiting
> confirmation for the patient name.

So the word "bestätigt" / "confirmed" in the question pulls it to the label `confirm_yes`.

## How big it is

I asked 14 questions about the form (7 German, 7 English) at 10 points of the form each, 140 turns, over HTTP
against the running app.

| The matcher took it for | How many | What the caller hears | Harm |
|---|---|---|---|
| A question about the form (right) | 97 | An answer, then the step's question | None |
| A yes | 19 | "Danke." and the next step | Yes, in 10 of them |
| An outside request | 13 | "Dabei kann ich im Moment leider nicht helfen." and the same question again | No, but no answer |
| An answer without a value | 10 | "Das habe ich leider nicht verstanden." and the same question again | No, but no answer |
| The end of the call | 1 | The goodbye, and the form is dropped | Yes |

About the 19:

- **They are all one question.** "Ist der Termin damit schon bestätigt?" was a yes in 9 of 10, "Is the appointment
  confirmed now?" in 10 of 10.
- **10 of them were asked while a value was being read back.** Those confirm the value.
- **The other 9 were asked when nothing was waiting for a yes.** There a yes changes nothing and the pending
  question is asked again.

Per question, how often it was taken for a question about the form (of 10 points):

| Question | Right | Taken for |
|---|---|---|
| Haben Sie den Termin schon gebucht? | 10 | |
| Welchen Namen haben Sie notiert? | 10 | |
| Was haben Sie bisher notiert? | 10 | |
| Welches Geburtsdatum haben Sie notiert? | 10 | |
| Welche Angaben brauchen Sie noch von mir? | 5 | an answer without a value (5) |
| Wofür brauchen Sie das Geburtsdatum? | 6 | an outside request (4) |
| **Ist der Termin damit schon bestätigt?** | **1** | **a yes (9)** |
| Which name did you write down? | 10 | |
| What have you recorded so far? | 10 | |
| What else do you need from me? | 9 | the end of the call (1) |
| Did you book the appointment already? | 8 | an outside request (2) |
| Which date of birth do you have? | 5 | an answer without a value (5) |
| Why do you need the date of birth? | 3 | an outside request (7) |
| **Is the appointment confirmed now?** | **0** | **a yes (10)** |

## It is not new

Task 1 did not cause this. The matcher's prompt is the same as before. Only the conversation history in it
changed, because the replies in it are code's sentences now.

I put the same 140 questions to the matcher twice: once with the history the old build had at those points, once
with the history it has now.

| | Old build's history | History now |
|---|---|---|
| Taken for a question | 89 of 140 | 96 of 140 |
| The "confirmed" question taken for a yes | 19 of 20 | 19 of 20 |
| Turns that change the form or the call wrongly | 12 | 11 |

It was found only now because nobody had asked these questions before. The suites have three questions about the
form, and none has "bestätigt" in it.

---

## Where a fix goes

In the vocabulary file, `app/config/llm_vocabulary.toml`. No Rust code has to change for the first three ideas.

What the form's matcher is told today:

**Its rules** (`[machines.form_intent_matcher]`)

1. Classify only the latest `<utterance>`. Use `<conversation_history>` to understand short or elliptical answers
   such as "yes", "the second one" or a bare name
2. Look at the state of the `current_field` of `<form_state>` first: when it is `queued` the caller was asked for
   its value, when it is `awaiting_confirmation` the caller was asked whether its value is right
3. Use `confirm_yes` only for a clear agreement. A word that neither agrees, denies nor gives a value is
   `unsupported`
4. A request that is not about this form, such as the weather, an exchange rate, a question about the calendar or
   another booking, is `unsupported`

**The two descriptions that matter** (`[intents.<label>]`)

- `confirm_yes`: Agreement when the `current_field` of `<form_state>` is `awaiting_confirmation`
- `get_information[form_information]`: A question about the form or the values the caller gave, such as "what
  name did you record" or "did you book it"

## Ideas to try, cheapest first

| | Idea | What changes | Worry |
|---|---|---|---|
| A | A rule that a question is never a yes | One more rule, or rule 3 made longer: "An utterance that asks something is never `confirm_yes`. A question about the form, its values or the booking is `get_information[form_information]`" | A real yes that sounds like a question ("Ja, stimmt, oder?") |
| B | More examples in the description of the form question | "…such as "what name did you record", "did you book it", "is it confirmed" or "what else do you need"" | Examples can pull other utterances to this label |
| C | The description of `confirm_yes` says what it is not | "…An agreement, never a question" | Changing this description went wrong before, see "What not to repeat" |
| D | The matcher says first whether the utterance asks something | A new key `is_question` in front of `caller_intent` in its answer. Code then never confirms on a question | The matcher's answer and prompt change, so everything about it has to be measured again. This one needs Rust code |
| E | No `confirm_yes` in the menu when nothing waits for a yes | The menu the matcher sees depends on the state of the current field | Fixes only the 9 harmless turns, not the 10 that confirm a value. Needs Rust code |

A, B and C can be tried without building anything: take a logged prompt, change the one text in it, ask the
model again.

A caller's question does not always end in "?". Speech recognition often leaves it out. So a rule has to speak
about asking, not about the question mark, and code cannot simply look for one.

---

## How to measure it

One changed text can fix these 20 turns and break others. So every idea is measured on four sets, and each set
is asked once without the change first, because the model does not always answer the same way twice.

| Set | What it is | What to count |
|---|---|---|
| 1. The 140 questions | 14 questions at 10 points of the form, German and English | How many are taken for a question. How many "confirmed" questions are a yes |
| 2. Real agreements | Every logged form turn that was a plain yes: "Ja", "Ja, das ist richtig.", "Genau.", "Stimmt.", "Yes", "Yes, that's correct." | How many are still `confirm_yes`. This must not drop |
| 3. Everything else | Every other logged prompt of the form's matcher: values, corrections, denials, outside requests, cancel, repeat | How many answers change against the run without the change, and whether each change is for the better |
| 4. The suites | `evals/run.py -r 2` | The `Intent` check, and the goldens about questions |

**How a replay works**

- The app logs every `Machine prompt` and `Machine answered`. The matcher's records have `output="ExtractedFormIntent"`.
- A prompt is everything up to `<utterance>`. The role and the rules are at its top, the history and the context
  after them.
- To try a rule: swap the text in the top part, keep the rest, and send it to vLLM again with the same JSON schema
  (`machine_reasoning` a string, `caller_intent` one of the eleven labels of the form menu).
- To ask a new question at a point of the form: take a prompt logged at that point and swap the `<utterance>`.
- No script for this is in the repo. The ones behind the numbers above were written for that one measurement.

The logs rotate. The 140 questions are in `logs/app.log.2026-10-06`, asked between 17:29 and 17:32 UTC. If that
file is gone, they have to be asked again: the 14 questions above, at every step of the suites' walk through the
form (`FORM_WALK` in `evals/suite.py`), from a copy of the walk's session at that step (`FormStates.at`).

**When an idea is good enough**

- The "confirmed" question is a yes in at most 2 of 20 (19 today).
- At least 105 of the 140 questions are taken for a question (97 today).
- Set 2: no real agreement is lost.
- Set 3: no more answers get worse than get better.
- Set 4: the `Intent` check is not lower than before (294/304 German, 144/148 English).

## What to build if an idea passes

- The changed text in `app/config/llm_vocabulary.toml`, then `./update-app.sh`.
- Goldens in `evals/test_form.py` for the question that started this, in German and English, asked while the name
  is being read back: the intent is `get_information[form_information]` and the form is unchanged.
- One line in `context.md` under the known gaps (gap 5), or its removal.

## What not to repeat

These were measured on this model before (see `context.md`, Verification).

- **Rewording `confirm_yes` to mention "yes, but"** made 1 to 2 of 59 corrections come out as `confirm_yes`. That
  description was left alone on purpose. Idea C touches it again, so it comes last of the three.
- **A rule that looked right on one call was worse on 139.** A single replay picked a matcher rule that the full
  run showed to be worse (7 wrong against 4). So never judge a rule by a few turns.
- **Two runs of the suites do not rank two wordings.** The goldens of one point of the form pass or fail
  together, and the clock is in every prompt, so a close call can flip between two runs of the same build.
- **The matcher's confidence does not help.** In the last run of the suites 8 of its 14 wrong intents had a
  confidence of 0.99 or more, and a cut-off under the lowest right one would have caught 3.

## Not part of this task

They show in the same 140 turns and can be counted in the same runs, but each is its own problem:

- "Welche Angaben brauchen Sie noch von mir?" and "Which date of birth do you have?" taken for an answer without
  a value, in half of the turns.
- "What else do you need from me?" taken for the end of the call, once.
- "Why do you need the date of birth?" turned down as an outside request. That may even be right: it is not a
  question about what the form holds.
