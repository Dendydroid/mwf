# evals

[DeepEval](https://deepeval.com) suites for the phone assistant. They talk to the running app over HTTP the way
the phone side does, check every turn part by part, and write a report: which part of a turn fails, for which
capability and language, and where the time of a turn goes.

German comes first, English second. There are no goldens in other languages.

## Run

The app, vLLM and Redis have to be up (`docker compose up -d`, and `./update-app.sh` after a change to the app).

```bash
# once, from the project root (made with Python 3.11)
python -m venv evals/.venv
evals/.venv/Scripts/python -m pip install -r evals/requirements.txt

evals/.venv/Scripts/python evals/run.py                      # every suite, about 5 minutes
evals/.venv/Scripts/python evals/run.py -r 3                 # every golden three times
evals/.venv/Scripts/python evals/run.py test_form.py         # one suite
evals/.venv/Scripts/python evals/run.py test_form.py -k de   # and whatever else pytest takes
evals/.venv/Scripts/python evals/report.py                   # the report of the latest run again
```

`run.py` calls `deepeval test run` and then writes `results/report_<time>.md` next to DeepEval's own results
file, `results/test_run_<time>.json`. It prints the report's summary and exits with 1 when a golden failed.

One run is one sample: vLLM does not always give the same answer at temperature 0, so compare two versions of a
prompt with `-r 3` or more and read the pass rates.

| Variable | Default | |
|---|---|---|
| `EVAL_APP_URL` | `http://localhost:8080` | the app |
| `EVAL_REDIS_URL` | `redis://localhost:6379` | the app's Redis, to copy a call's session |
| `EVAL_APP_LOG_DIR` | `../logs` | the app's log files, for the time each machine took |
| `EVAL_TURN_BUDGET_MS` | `2000` | what a turn may take before `Turn Latency` fails |
| `EVAL_JUDGE_MODEL` | none | the judge of the LLM-judged metrics, see below |
| `EVAL_LLM_URL` | `http://localhost:8000/v1` | vLLM, for `EVAL_JUDGE_MODEL=local` |

## What is tested

| Suite | Goldens | What |
|---|---|---|
| `test_main_menu.py` | 23 de, 14 en | Weather, exchange rate, starting the form (also from symptoms alone), the calendar refusal, repeat, end call, transfer, and requests that are close to a supported one but not the same |
| `test_hints.py` | 8 de, 6 en | What the caller says before the form: taken as hints, kept over turns, put into the started form to be confirmed, left out when a validator refuses it |
| `test_form.py` | 60 de, 25 en | One answer at one point of the doctor form: a value, yes, no, "no, it is …", "yes, but …", a value for the next field, the date and the time of the appointment said in one answer, a question about the form, an outside request, repeat, cancel, transfer, end call, the date-of-birth validator, completing the form, and the main menu after a completed form, where the hints hold its values and a second form starts with them |
| `test_conversations.py` | 9 de, 3 en | Whole calls: booking with full sentences, with short answers, with hints, with corrections, three rejections and the offer of a human, interruptions, a value the caller points to, a change of language |
| `test_api.py` | 10 | Bad requests, the keys of the response, the session and its sliding expiry, one turn of a call at a time, a turn that fails on the app's side, what becomes of a completed form |

The form suite does not play a call up to the point it tests. `FormStates` in `suite.py` takes one call per
language through the form once and copies its session in Redis after every step (`COPY call_id.<a> call_id.<b>`).
Each golden starts from a copy of the state it names, so every answer is tried at exactly the same point.

A golden that shows a gap `context.md` already lists carries `known_gap`. It is run and reported, but it is
marked `flaky` for DeepEval, so it does not fail the run.

## The checks

`metrics.py`. A golden says what it expects (`suite.py` lists the keys), and a check compares one part of the
turn with it. None of them asks a model, so the same answer always gets the same score.

| Check | Part of the turn | Passes when |
|---|---|---|
| Language Detection | lingua | `language_detected` is the caller's language |
| Hint Extraction | context extractor, main menu | the hints of the call are the expected ones |
| Intent | intent matcher | `selected_function` is the expected intent |
| Form State | context extractor of the form, flow code | the named fields have the expected state and value |
| Completed Form | the same | the form the turn completed has the expected values |
| Call Action | flow code | `action` is the expected one |
| Response Context | flow code | the formulator was told what it should be told |
| Reply Language | response formulator | lingua finds the reply in the caller's language |
| Reply Speakable | response formulator | no markup, snake_case name, ISO date, emoji or letter of another script |
| Reply Says | response formulator | the reply mentions what the golden lists and nothing it forbids |
| Reply Asks | response formulator | at most one question, and it is the form's next step: the field it is on, or the confirmation of the value read back |
| No Early Booking | response formulator | the reply does not say the appointment is booked while the form is open |
| Turn Latency | speed | the turn took no longer than the budget |
| Repeated Sentences | response formulator | no reply of a conversation says a whole sentence of the reply before it again |
| Facts | API and session | what the test worked out itself holds, e.g. the status of a bad request |

`Reply Says`, `Reply Asks` and `No Early Booking` go by words, in German and English. A failure of `Reply Asks`
has been a real one every time so far: of the 177 different replies of the first run, read by hand, the 23 it
failed were all wrong, and 1 of the 154 it passed was wrong too. What it cannot tell is whether a reply is
well worded, or whether a date read back in words is the right one.

## Judged metrics

For that there are LLM-judged metrics, added to every test case when `EVAL_JUDGE_MODEL` is set:
`Follows Instruction` (GEval: the reply against the response context), `Faithfulness`, and for a conversation
`Form Manners` (ConversationalGEval) and `Knowledge Retention`.

```bash
EVAL_JUDGE_MODEL=gpt-4.1 OPENAI_API_KEY=... evals/.venv/Scripts/python evals/run.py
EVAL_JUDGE_MODEL=claude-opus-5-5 ANTHROPIC_API_KEY=... evals/.venv/Scripts/python evals/run.py
EVAL_JUDGE_MODEL=local evals/.venv/Scripts/python evals/run.py      # the project's own vLLM
```

They are off by default because the only judge at hand without a key is the app's own Qwen2.5-7B, and it is not
good enough. On replies labelled by hand (2026-10-06, German and English, answers held to the metric's JSON
schema) it agreed with the label in 10 of 13 with GEval, 12 of 16 with PromptAlignment, and 20 of 36 with narrow
yes-or-no questions as a DAG metric. It called a correct read-back wrong and a reply that skipped a confirmation
right. With `local` the metrics run, but their scores should not decide anything. With a stronger model they have
not been run yet.

## The report

- **What to look at first**: the pass rates, the weakest checks, the slowest step.
- **Which part of a turn fails**: per check and language, counted per turn, so a conversation counts with every turn.
- **Where a failing turn fails first**: a turn with a wrong intent also fails its form state and reply. The first
  failed check, in the order a turn runs, is the part to look at.
- **Which capability fails**: goldens per suite, capability and language.
- **Where the time goes**: the context extractor, the intent matcher, the intent handler (state change,
  validators, the weather or rate request) and the response formulator of the main menu and of the form, with
  the size of their prompts and answers.
- **Does the matcher's confidence point at its mistakes**: the confidence of right and wrong intents.
- **Known gaps**, and every **failed golden** with what the caller said and heard.

The report works the checks out again from the turns in the results file, so after a change to a check
`report.py` shows its effect on an old run without calling the app.

## Adding a golden

```python
golden("birth-no-other-year-de", "Nein, 1992.", state="dob_awaiting", lang="de", capability="correct a value",
       intent=CORRECT, form={"date_of_birth": ["awaiting_confirmation", "1992-06-13"]}),
```

The name, what the caller says, and what the turn has to come to. In the form suite `state` is the point of the
form it is said at. A value is `None`, a text it has to equal, or `has("a", ("b", "c"))` for a value that holds
`a` and one of `b` and `c`. A conversation is a list of such lines, see `test_conversations.py`.

## Limits

- The steps of a turn are timed from the app's log (`Machine prompt` to `Machine answered`), which is the app's
  wall clock. In the Docker VM that clock runs about 3% fast against this machine's and is set back now and
  then. A turn it was set back in is left out of the step times. The time the caller waits is measured here and
  is not affected.
- Weather and rate are fetched from the live services, so those goldens need the network.
- Whether a completed form arrives at the other project is not checked: without `FORM_SUBMIT_URL` the app sends
  nothing, and the suite only sees in the log that the form reached the submitter.
- `-n` runs goldens in parallel. They then share the GPU, and the timings say nothing.
- The suites leave their calls in Redis until they expire (an hour, six for the copied form states), and each
  of the three goldens of a failed turn writes a prompt of about 130 kB into the app's log.
