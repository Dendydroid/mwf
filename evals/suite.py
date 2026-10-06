"""
    What the suites share: how a golden says what it expects, how a turn or a whole call becomes a
    DeepEval test case, and the walk through the doctor form that the form suite starts its turns from.

    What a golden can expect of a turn (see the checks in metrics.py):

    lang         the language the caller speaks, "de" or "en"                         (always)
    capability   what of the assistant the golden is about, for the report            (always)
    intent       the intent the matcher has to choose, or a list when several are right
    action       "continue" (the default), "end_call" or "transfer_to_human"
    hints        {hint: value} for the hints the call has to hold after the turn
    form         {field: [state, value]} for the form after the turn, "none" for no form in progress,
                 "unchanged" for the form as it was before the turn
    completed    {field: value} for the form the turn completed
    context      parts the response context, which is what the formulator is told, has to hold
    says         what the reply has to mention, each as a list of the wordings that count
    never_says   the same for what it must not mention
    asks         "nothing", "anything_else" or "any", in place of the next step of the form
    failed       True for a turn that is made to fail on the app's side
    known_gap    the gap of context.md the golden shows. Its failure is reported and does not fail the run

    A value is None for no value, a text it has to equal, or `has(...)`.

    What code says as it is comes from the app's vocabulary, so a golden names the text by its key
    (`phrase`, `refusal`, `form_says`, `ask`, `confirm`) and `says` takes it word for word.
"""
import tomllib
from pathlib import Path

import pytest
from deepeval import assert_test
from deepeval.dataset import ConversationalGolden, Golden
from deepeval.test_case import ConversationalTestCase, LLMTestCase, ToolCall, Turn

from assistant import Call
from judge import judge
from metrics import (
    Facts,
    conversation_metrics,
    form_problems,
    judged_conversation_metrics,
    judged_turn_metrics,
    turn_metrics,
    value_of,
)

JUDGE = judge()

ASSISTANT_ROLE = (
    "A phone assistant that tells the weather in Berlin and the euro to hryvnia rate, and books a doctor's "
    "appointment by filling in a form with the caller, one confirmed value at a time."
)


def has(*parts):
    """A value that holds every part. A part given as a tuple is any one of its wordings."""
    return {"has": [[part] if isinstance(part, str) else list(part) for part in parts]}


ANY = has()
NO_HINTS = {
    "caller_full_name": None,
    "patient_full_name": None,
    "date_of_birth_iso_8601": None,
    "appointment_spoken_date": None,
    "appointment_spoken_time": None,
}


# The texts the app says as they are, from the file the app reads them from
VOCABULARY = tomllib.loads(
    (Path(__file__).resolve().parent.parent / "app" / "config" / "llm_vocabulary.toml").read_text(encoding="utf-8")
)
DOCTOR_FORM = VOCABULARY["forms"]["doctor_appointment"]


def phrase(name, lang):
    """A sentence code says in any form, e.g. `phrase("thanks", "de")`. Like the four below it is what
    `says` takes for one thing the reply has to hold: the wordings that count, here the only one."""
    return [VOCABULARY["phrases"][name][lang]]


def refusal(name, lang):
    """What a validator tells the caller."""
    return [VOCABULARY["validation"][name][lang]]


def form_says(what, lang):
    """What the doctor form says when it is `started`, `completed` or `cancelled`."""
    return [DOCTOR_FORM[what][lang]]


def ask(field, lang):
    """The question that asks for a field of the doctor form."""
    return [DOCTOR_FORM["fields"][field]["ask"][lang]]


def confirm(field, value, lang):
    """The question that reads `value` back for a field of the doctor form."""
    return [DOCTOR_FORM["fields"][field]["confirm"][lang].replace("{value}", value)]


def golden(name, utterance, **expect):
    return Golden(name=name, input=utterance, additional_metadata=expect)


def conversation(name, scenario, lang, capability, steps, known_gap=None):
    """`steps` are the caller's utterances in order, each with what its turn is expected to come to."""
    return ConversationalGolden(
        name=name,
        scenario=scenario,
        turns=[Turn(role="user", content=utterance) for utterance, _ in steps],
        additional_metadata={
            "lang": lang,
            "capability": capability,
            "known_gap": known_gap,
            "steps": [expect for _, expect in steps],
        },
    )


def as_expected(form):
    """A form as the expectation that it is exactly like this."""
    if form is None:
        return "none"

    return {field["name"]: [field["state"], value_of(field)] for field in form["fields"]}


def recorded(turn):
    """A turn as the metrics and the report read it: the app's answer and what the turn took."""
    return {
        **turn.body,
        "utterance": turn.utterance[:300],
        "call_id": turn.call_id,
        "latency_ms": turn.latency_ms,
        "stages": turn.stages,
    }


def check_turn(suite, golden, turn, form_before=None):
    expect = dict(golden.additional_metadata)
    if expect.get("form") == "unchanged":
        expect["form"] = as_expected(form_before)

    data = recorded(turn)
    context = [turn.response_context] if turn.response_context else None
    test_case = LLMTestCase(
        name=golden.name,
        input=golden.input,
        actual_output=turn.answer,
        # What the formulator was told to say is what its reply is judged against
        context=context,
        retrieval_context=context,
        tools_called=[ToolCall(name=turn.intent)] if turn.intent else None,
        completion_time=turn.latency_ms / 1000,
        tags=[suite, expect["capability"], expect["lang"]],
        flaky=bool(expect.get("known_gap")),
        metadata={"suite": suite, "expect": expect, "turn": data},
    )

    metrics = turn_metrics(expect, data)
    if JUDGE:
        metrics += judged_turn_metrics(JUDGE, data)

    assert_test(test_case, metrics)


def check_conversation(suite, golden):
    about = golden.additional_metadata
    call = Call()
    turns, heard = [], []

    for caller, expect in zip(golden.turns, about["steps"]):
        expect = {"lang": about["lang"], "capability": about["capability"], **expect}
        form_before = call.turns[-1].form if call.turns else None
        turn = call.say(caller.content)
        if expect.get("form") == "unchanged":
            expect["form"] = as_expected(form_before)

        turns.append({"expect": expect, "turn": recorded(turn)})
        heard += [caller, Turn(role="assistant", content=turn.answer, latency_ms=turn.latency_ms)]

    test_case = ConversationalTestCase(
        name=golden.name,
        scenario=golden.scenario,
        chatbot_role=ASSISTANT_ROLE,
        turns=heard,
        tags=[suite, about["capability"], about["lang"]],
        flaky=bool(about["known_gap"]),
        metadata={
            "suite": suite,
            "expect": {"lang": about["lang"], "capability": about["capability"], "known_gap": about["known_gap"]},
            "turns": turns,
        },
    )

    metrics = conversation_metrics(turns)
    if JUDGE:
        metrics += judged_conversation_metrics(JUDGE)

    assert_test(test_case, metrics)


def check_facts(suite, name, capability, sent, got, facts):
    """For what is not a turn of a call: `facts` says what has to hold, each with whether it does."""
    test_case = LLMTestCase(
        name=name,
        input=sent,
        actual_output=got,
        tags=[suite, capability],
        metadata={"suite": suite, "expect": {"lang": "any", "capability": capability}, "facts": facts},
    )

    assert_test(test_case, [Facts()])


# --------------------------------------------------------------------------------------------------
#  The walk through the doctor form: the state after each line, what the caller says to get there,
#  and what the form has to look like then
# --------------------------------------------------------------------------------------------------

FORM_WALK = {
    "de": [
        ("name_queued", "Ich möchte einen Arzttermin vereinbaren.",
         {"patient_name": ["queued", None]}),
        ("name_awaiting", "Der Patient heißt Hans Müller.",
         {"patient_name": ["awaiting_confirmation", "Hans Müller"]}),
        ("dob_queued", "Ja, das ist richtig.",
         {"patient_name": ["completed", "Hans Müller"], "date_of_birth": ["queued", None]}),
        ("dob_awaiting", "Er ist am 13. Juni 1991 geboren.",
         {"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
        ("reason_queued", "Ja, das stimmt.",
         {"date_of_birth": ["completed", "1991-06-13"], "reason": ["queued", None]}),
        ("reason_awaiting", "Er hat starke Kopfschmerzen.",
         {"reason": ["awaiting_confirmation", has("kopfschmerzen")]}),
        ("date_queued", "Ja, genau.",
         {"reason": ["completed", ANY], "appointment_date": ["queued", None]}),
        ("date_awaiting", "Am liebsten nächsten Samstag.",
         {"appointment_date": ["awaiting_confirmation", ANY]}),
        ("time_queued", "Ja, das passt.",
         {"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]}),
        ("time_awaiting", "Am liebsten am Vormittag.",
         {"appointment_time": ["awaiting_confirmation", ANY]}),
        ("completed", "Ja, das ist gut so.", "none"),
    ],
    "en": [
        ("name_queued", "I would like to book a doctor's appointment.",
         {"patient_name": ["queued", None]}),
        ("name_awaiting", "The patient's name is John Smith.",
         {"patient_name": ["awaiting_confirmation", "John Smith"]}),
        ("dob_queued", "Yes, that's correct.",
         {"patient_name": ["completed", "John Smith"], "date_of_birth": ["queued", None]}),
        ("dob_awaiting", "He was born on June 13th, 1991.",
         {"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
        ("reason_queued", "Yes, that's right.",
         {"date_of_birth": ["completed", "1991-06-13"], "reason": ["queued", None]}),
        ("reason_awaiting", "He has a bad headache.",
         {"reason": ["awaiting_confirmation", has("headache")]}),
        ("date_queued", "Yes, exactly.",
         {"reason": ["completed", ANY], "appointment_date": ["queued", None]}),
        ("date_awaiting", "Next Saturday, please.",
         {"appointment_date": ["awaiting_confirmation", ANY]}),
        ("time_queued", "Yes, that works.",
         {"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]}),
        ("time_awaiting", "In the morning, please.",
         {"appointment_time": ["awaiting_confirmation", ANY]}),
        ("completed", "Yes, that is fine.", "none"),
    ],
}

WALK_ATTEMPTS = 3


class FormStates:
    """One call per language is taken through the doctor form once, and its session is copied in Redis
    after every line. A test starts from a copy of the state it needs, so every answer is tried at
    exactly that point of the form, and the turns before it are not played again."""

    def __init__(self):
        self.walks = {}

    def at(self, lang, state):
        """A fresh call that is in `state`, and the form it has there."""
        if lang not in self.walks:
            self.walks[lang] = self.walk(lang)

        states, problem = self.walks[lang]
        if state not in states:
            pytest.fail(f"The walk did not get a {lang} call to {state}: {problem}", pytrace=False)

        snapshot, form = states[state]

        return snapshot.copy(), form

    def walk(self, lang):
        furthest = ({}, None)

        for _ in range(WALK_ATTEMPTS):
            call, states, problem = Call(), {}, None

            for state, utterance, expected in FORM_WALK[lang]:
                turn = call.say(utterance)
                problems = form_problems(expected, turn.form)
                if problems:
                    problem = f'after "{utterance}": {"; ".join(problems)}'
                    break

                states[state] = (call.copy(), turn.form)

            if problem is None:
                return states, None
            if len(states) >= len(furthest[0]):
                furthest = (states, problem)

        return furthest
