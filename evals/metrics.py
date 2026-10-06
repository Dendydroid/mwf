"""
    The metrics of the suites.

    A check looks at one turn: `expect` is what the golden asks for, `turn` is the app's JSON answer with
    the turn's `latency_ms` added, and `reply` the sentence the caller heard. It returns its score (0 to 1)
    with what it found, or None when the golden asks nothing of it. None of the checks needs a judge
    model, so the same answer always gets the same score. `STAGES` says which part of a turn each check
    is about, in the order a turn runs them.

    `Check` makes one of them a DeepEval metric for a single turn, `EachTurn` for a whole conversation.
    The LLM-judged metrics at the bottom are only added when a judge is configured, see judge.py.
"""
import os
import re
import unicodedata

from deepeval.metrics import (
    BaseConversationalMetric,
    BaseMetric,
    ConversationalGEval,
    FaithfulnessMetric,
    GEval,
    KnowledgeRetentionMetric,
)
from deepeval.test_case import MultiTurnParams, SingleTurnParams
from lingua import Language, LanguageDetectorBuilder

# What a caller should not have to wait longer than for one answer
TURN_BUDGET_MS = float(os.getenv("EVAL_TURN_BUDGET_MS", "2000"))

CHECKS = {}
STAGES = {}


def check(name, stage):
    def register(function):
        CHECKS[name] = function
        STAGES[name] = stage

        return function

    return register


# --------------------------------------------------------------------------------------------------
#  Expected values: None for no value, a text the value has to equal, or `has(...)` parts it has to hold
# --------------------------------------------------------------------------------------------------


def normalized(text):
    return " ".join(str(text).lower().split()).strip(" .,!?")


def matches(expected, actual):
    if expected is None:
        return actual is None
    if actual is None:
        return False
    if isinstance(expected, dict):
        return all(any(normalized(option) in normalized(actual) for option in options) for options in expected["has"])

    return normalized(expected) == normalized(actual)


def described(expected):
    if isinstance(expected, dict):
        return "a value with " + " and ".join("/".join(options) for options in expected["has"]) if expected["has"] else "any value"

    return repr(expected)


def value_of(field):
    """A field's value without its kind: {"date": "1991-06-13"} is 1991-06-13."""
    value = field.get("value")

    return None if value is None else str(next(iter(value.values())))


def current_field(form):
    return next((field for field in form["fields"] if field["state"] != "completed"), None)


def form_problems(expected, form):
    """`expected` is "none" for no form in progress, or the fields that matter as {name: [state, value]}."""
    if expected == "none":
        return [] if form is None else ["a form is still in progress"]
    if form is None:
        return ["no form is in progress"]

    fields = {field["name"]: field for field in form["fields"]}
    problems = []
    for name, (state, value) in expected.items():
        field = fields.get(name)
        if field is None:
            problems.append(f"{name} is not in the form")
        elif field["state"] != state or not matches(value, value_of(field)):
            problems.append(f"{name} is {field['state']} with {value_of(field)!r}, expected {state} with {described(value)}")

    return problems


# --------------------------------------------------------------------------------------------------
#  Checks, in the order a turn runs
# --------------------------------------------------------------------------------------------------


@check("Language Detection", "language detector (lingua)")
def language_detection(expect, turn, reply):
    detected = turn.get("language_detected")

    return detected == expect["lang"], f"detected {detected}, the caller spoke {expect['lang']}"


@check("Hint Extraction", "context extractor (main menu)")
def hint_extraction(expect, turn, reply):
    if "hints" not in expect:
        return None

    hints = turn.get("hint_map") or {}
    wrong = [
        f"{name} is {hints.get(name)!r}, expected {described(expected)}"
        for name, expected in expect["hints"].items()
        if not matches(expected, hints.get(name))
    ]

    return 1 - len(wrong) / len(expect["hints"]), "; ".join(wrong) or "every hint is as expected"


@check("Intent", "intent matcher")
def intent(expect, turn, reply):
    if "intent" not in expect:
        return None

    allowed = expect["intent"] if isinstance(expect["intent"], list) else [expect["intent"]]
    matched, confidence = turn.get("selected_function"), turn.get("confidence")
    sure = f" with confidence {confidence:.4f}" if confidence is not None else ""

    return matched in allowed, f"matched {matched}{sure}, expected {' or '.join(map(str, allowed))}"


@check("Form State", "context extractor (form) and flow code")
def form_state(expect, turn, reply):
    if "form" not in expect:
        return None

    problems = form_problems(expect["form"], turn.get("form"))
    listed = 1 if expect["form"] == "none" else len(expect["form"])

    return 1 - len(problems) / max(listed, len(problems)), "; ".join(problems) or "the form is as expected"


@check("Completed Form", "context extractor (form) and flow code")
def completed_form(expect, turn, reply):
    if "completed" not in expect:
        return None

    form = (turn.get("hint_map") or {}).get("last_filled_out_form")
    expected = {name: ["completed", value] for name, value in expect["completed"].items()}
    problems = form_problems(expected, form) if form else ["no completed form was kept for the call"]

    return 1 - len(problems) / max(len(expected), len(problems)), "; ".join(problems) or "completed with the expected values"


@check("Call Action", "flow code")
def call_action(expect, turn, reply):
    expected, action = expect.get("action", "continue"), turn.get("action")

    return action == expected, f"action is {action}, expected {expected}"


@check("Response Context", "flow code")
def response_context(expect, turn, reply):
    if "context" not in expect:
        return None

    context = turn.get("response_context") or ""
    missing = [fragment for fragment in expect["context"] if fragment not in context]

    return 1 - len(missing) / len(expect["context"]), (
        f"missing {missing} in: {context}" if missing else "the formulator was told what was expected"
    )


REPLY_DETECTOR = LanguageDetectorBuilder.from_languages(
    Language.ENGLISH, Language.GERMAN, Language.RUSSIAN, Language.UKRAINIAN
).build()


@check("Reply Language", "response formulator")
def reply_language(expect, turn, reply):
    detected = REPLY_DETECTOR.detect_language_of(reply)
    code = detected.iso_code_639_1.name.lower() if detected else None

    return code == expect["lang"], f"the reply is in {code}, the caller spoke {expect['lang']}"


MARKUP = re.compile(r"[*#`_\[\]{}<>|\\\n]")
ISO_DATE = re.compile(r"\d{4}-\d{2}-\d{2}")
EMOJI = re.compile("[\U0001F000-\U0001FAFF☀-➿]")


@check("Reply Speakable", "response formulator")
def reply_speakable(expect, turn, reply):
    """The reply is read out by a speech synthesizer: plain words in the caller's script, nothing else."""
    problems = []
    if not reply.strip():
        problems.append("the reply is empty")
    if MARKUP.search(reply):
        problems.append(f"markup or a snake_case name: {MARKUP.findall(reply)}")
    if ISO_DATE.search(reply):
        problems.append(f"a date not written the way it is spoken: {ISO_DATE.findall(reply)}")
    if EMOJI.search(reply):
        problems.append("an emoji")
    if expect["lang"] in ("de", "en"):
        foreign = sorted({c for c in reply if c.isalpha() and not unicodedata.name(c, "").startswith("LATIN")})
        if foreign:
            problems.append(f"letters of another script: {''.join(foreign)}")

    return not problems, "; ".join(problems) or "plain speakable text"


def said(options, reply):
    return any(option.lower() in reply.lower() for option in options)


@check("Reply Says", "response formulator")
def reply_says(expect, turn, reply):
    """`says` lists what the reply has to mention, each as the wordings that count; `never_says` what it must not."""
    if "says" not in expect and "never_says" not in expect:
        return None

    missing = [options for options in expect.get("says", []) if not said(options, reply)]
    forbidden = [options for options in expect.get("never_says", []) if said(options, reply)]
    problems = [f"does not mention {'/'.join(options)}" for options in missing]
    problems += [f"mentions {'/'.join(options)}" for options in forbidden]
    asked = len(expect.get("says", [])) + len(expect.get("never_says", []))

    return 1 - len(problems) / asked, "; ".join(problems) or "says what was expected"


# How a reply asks for a field, in German and in English, as beginnings of words
ASKS_FOR = {
    "patient_name": ["name", "heiß"],
    "date_of_birth": ["geburt", "geboren", "birth", "born"],
    "reason": ["grund", "anliegen", "warum", "weshalb", "weswegen", "worum", "wofür", "beschwerde", "anlass",
               "reason", "why", "what brings", "purpose"],
    # "Termin" and "appointment" are in the question for the date and in the one for the time
    "appointment_date": ["wann", "datum", "tag", "when", "date", "day"],
    "appointment_time": ["wann", "uhr", "zeit", "when", "time", "o'clock"],
}

# A yes or no can be asked for without a question mark: "Bestätigen Sie, dass …", "Please confirm that …"
CONFIRM_REQUEST = re.compile(r"\b(bestätigen|confirm)\b", re.IGNORECASE)

# A sentence ends at . ! ? unless the number of a day stands before it, as in "13. Juni"
SENTENCE_END = re.compile(r"(?<!\b\d)(?<!\b\d\d)[.!?]+\s+")


def asked_sentence(reply):
    sentences = [sentence for sentence in SENTENCE_END.split(reply.strip()) if sentence]
    questions = [sentence for sentence in re.split(r"(?<=\?)\s+", reply.strip()) if "?" in sentence]
    if questions:
        return SENTENCE_END.split(questions[-1])[-1]

    return sentences[-1] if sentences else ""


def asks_for(field, sentence):
    return any(re.search(rf"\b{re.escape(word)}", sentence, re.IGNORECASE) for word in ASKS_FOR.get(field, []))


def reads_back(field, reply):
    """Whether the value the caller is asked to confirm is in the reply, as far as words can tell."""
    kind, value = field["kind"], value_of(field)
    if kind == "string":
        words = re.findall(r"\w+", value.lower())
        words = [word for word in words if len(word) >= 4] or words
        found = [word for word in words if word[: max(4, len(word) - 2)] in reply.lower()]

        return len(found) >= (len(words) if len(words) <= 3 else len(words) / 2)
    if kind == "date":
        # A year written in digits has to be the right one. In words it cannot be told here
        years = re.findall(r"\b\d{4}\b", reply)

        return not years or value[:4] in years

    return True


def expected_step(expect, turn):
    if expect.get("asks"):
        return expect["asks"], None
    # A repeated reply is as good as the one it repeats
    if turn.get("selected_function") == "repeat":
        return "any", None
    if turn.get("action") != "continue":
        return "nothing", None
    if not turn.get("form"):
        return "any", None

    field = current_field(turn["form"])
    if field is None:
        return "any", None

    return ("confirm" if field["state"] == "awaiting_confirmation" else "ask"), field


@check("Reply Asks", "response formulator")
def reply_asks(expect, turn, reply):
    """One question at most, and it is the next step: the field the form is on, or the confirmation of
    the value it holds. Told by the words of the question, so a pass is likely and not certain."""
    step, field = expected_step(expect, turn)
    questions = reply.count("?")
    sentence = asked_sentence(reply)
    problems = []

    if questions > 1:
        problems.append(f"asks {questions} questions")
    if step == "nothing" and questions:
        problems.append("asks a question although it was told to ask nothing")
    if step == "anything_else" and not questions:
        problems.append("does not ask whether the caller needs anything else")
    if step == "ask" and not asks_for(field["name"], sentence):
        problems.append(f"does not ask for {field['name']}: \"{sentence}\"")
    if step == "confirm":
        others = [name for name in ASKS_FOR if name != field["name"] and asks_for(name, sentence)]
        if not questions and not CONFIRM_REQUEST.search(sentence):
            problems.append(f"does not ask whether {field['name']} is right")
        elif others and not asks_for(field["name"], sentence):
            problems.append(f"asks for {'/'.join(others)} instead of confirming {field['name']}: \"{sentence}\"")
        if not reads_back(field, reply):
            problems.append(f"does not read back {value_of(field)!r}")

    return not problems, "; ".join(problems) or f"asks what was expected ({step})"


BOOKING = re.compile(r"\b(termin|appointment|buchung|booking)", re.IGNORECASE)
CLAIMED = re.compile(
    r"\b(gebucht|eingetragen|festgelegt|ist vereinbart|wurde vereinbart|booked|is scheduled|has been scheduled|is confirmed|is set)\b",
    re.IGNORECASE,
)
NEGATED = re.compile(r"\b(nicht|kein|keinen|not|never)\b|n't", re.IGNORECASE)


@check("No Early Booking", "response formulator")
def no_early_booking(expect, turn, reply):
    """While the form is still open, the reply must not say the appointment is booked."""
    if not turn.get("form"):
        return None

    claims = [
        sentence for sentence in SENTENCE_END.split(reply)
        if BOOKING.search(sentence) and CLAIMED.search(sentence) and not NEGATED.search(sentence)
    ]

    return not claims, f"says it is booked while the form is still open: \"{claims[0]}\"" if claims else "does not say it is booked"


@check("Turn Latency", "speed")
def turn_latency(expect, turn, reply):
    latency_ms = turn.get("latency_ms")
    if latency_ms is None:
        return None

    return min(1.0, TURN_BUDGET_MS / latency_ms), f"{latency_ms:.0f} ms, the budget is {TURN_BUDGET_MS:.0f} ms"


def run_check(name, expect, turn):
    # A turn that failed on the app's side is answered with a fixed apology, which asks for no step
    if expect.get("failed") and name == "Reply Asks":
        return None

    return CHECKS[name](expect, turn, turn.get("answer") or "")


# --------------------------------------------------------------------------------------------------
#  The checks as DeepEval metrics
# --------------------------------------------------------------------------------------------------


class Check(BaseMetric):
    """One check of one turn. The test case's metadata holds `expect` and `turn`."""

    def __init__(self, check):
        self.check = check
        self.threshold = 1.0

    def measure(self, test_case, *args, **kwargs):
        outcome = run_check(self.check, test_case.metadata["expect"], test_case.metadata["turn"])
        score, self.reason = outcome if outcome else (1.0, "nothing was expected here")
        self.score = float(score)
        self.success = self.score >= self.threshold

        return self.score

    async def a_measure(self, test_case, *args, **kwargs):
        return self.measure(test_case)

    def is_successful(self):
        return bool(self.success)

    @property
    def __name__(self):
        return self.check


class EachTurn(BaseConversationalMetric):
    """One check over every turn of a conversation it applies to: the score is the share that passed.
    The test case's metadata holds `turns`, each with what `Check` reads."""

    def __init__(self, check):
        self.check = check
        self.threshold = 1.0

    def measure(self, test_case, *args, **kwargs):
        failed, applied = [], 0
        for number, data in enumerate(test_case.metadata["turns"], start=1):
            outcome = run_check(self.check, data["expect"], data["turn"])
            if outcome is None:
                continue
            applied += 1
            if float(outcome[0]) < 1.0:
                failed.append(f"turn {number} \"{data['turn']['utterance']}\": {outcome[1]}")

        self.score = 1 - len(failed) / applied if applied else 1.0
        self.reason = " | ".join(failed) or f"all {applied} turns passed"
        self.success = self.score >= self.threshold

        return self.score

    async def a_measure(self, test_case, *args, **kwargs):
        return self.measure(test_case)

    def is_successful(self):
        return bool(self.success)

    @property
    def __name__(self):
        return self.check


def sentences_of(reply):
    return {normalized(sentence) for sentence in SENTENCE_END.split(reply) if len(sentence.split()) >= 4}


class RepeatedSentences(BaseConversationalMetric):
    """A reply that says a whole sentence of the reply before it again, when the caller did not ask for
    it, is the formulator copying its own history instead of following the turn's instruction."""

    def __init__(self):
        self.threshold = 1.0

    def measure(self, test_case, *args, **kwargs):
        turns = test_case.metadata["turns"]
        repeated = []
        for number in range(1, len(turns)):
            before, now = turns[number - 1]["turn"], turns[number]["turn"]
            if now.get("selected_function") == "repeat":
                continue
            again = sentences_of(before.get("answer") or "") & sentences_of(now.get("answer") or "")
            if again:
                repeated.append(f"turn {number + 1} says again: \"{sorted(again)[0]}\"")

        self.score = 1 - len(repeated) / max(len(turns) - 1, 1)
        self.reason = " | ".join(repeated) or "no reply repeats the one before it"
        self.success = self.score >= self.threshold

        return self.score

    async def a_measure(self, test_case, *args, **kwargs):
        return self.measure(test_case)

    def is_successful(self):
        return bool(self.success)

    @property
    def __name__(self):
        return "Repeated Sentences"


STAGES["Repeated Sentences"] = "response formulator"


class Facts(BaseMetric):
    """For what is not a turn of a call, such as a bad request: the test works out the facts itself and
    puts them into the test case's metadata as {"what has to hold": true or false}."""

    def __init__(self):
        self.threshold = 1.0

    def measure(self, test_case, *args, **kwargs):
        facts = test_case.metadata["facts"]
        broken = [fact for fact, holds in facts.items() if not holds]
        self.score = 1 - len(broken) / len(facts)
        self.reason = "does not hold: " + "; ".join(broken) if broken else f"all {len(facts)} facts hold"
        self.success = self.score >= self.threshold

        return self.score

    async def a_measure(self, test_case, *args, **kwargs):
        return self.measure(test_case)

    def is_successful(self):
        return bool(self.success)

    @property
    def __name__(self):
        return "Facts"


STAGES["Facts"] = "API and session"


def turn_metrics(expect, turn):
    return [Check(name) for name in CHECKS if run_check(name, expect, turn) is not None]


def conversation_metrics(turns):
    applies = lambda name: any(run_check(name, data["expect"], data["turn"]) is not None for data in turns)

    return [EachTurn(name) for name in CHECKS if applies(name)] + [RepeatedSentences()]


# --------------------------------------------------------------------------------------------------
#  LLM-judged metrics, for the part of a reply that words alone cannot tell
# --------------------------------------------------------------------------------------------------

JUDGED_STAGE = "response formulator (judged)"
STAGES.update({
    "Follows Instruction [GEval]": JUDGED_STAGE,
    "Faithfulness": JUDGED_STAGE,
    "Form Manners [Conversational GEval]": JUDGED_STAGE,
    "Knowledge Retention": JUDGED_STAGE,
})


def judged_turn_metrics(model, turn):
    """`context` of the test case is the instruction the formulator got for the reply."""
    if not turn.get("response_context"):
        return []

    follows_instruction = GEval(
        name="Follows Instruction",
        evaluation_steps=[
            "The context is the instruction a phone assistant got for this reply, in English. The actual output "
            "is the reply it spoke to the caller, which may be in another language.",
            "Check that the reply does what the instruction asks for and gives every value the instruction "
            "says to state or to have confirmed, with the same value.",
            "When the instruction ends with a next step, check that the reply ends by asking for that step "
            "and asks for nothing else.",
            "Check that the reply does not do what the instruction forbids and claims nothing the instruction "
            "does not say, such as that a booking is done.",
            "A reply that follows the instruction in other words or in another language is fully correct. "
            "Wording and politeness do not matter.",
        ],
        evaluation_params=[SingleTurnParams.INPUT, SingleTurnParams.ACTUAL_OUTPUT, SingleTurnParams.CONTEXT],
        model=model,
        threshold=0.7,
    )

    # Facts the reply states have to come from the instruction: a temperature, a rate, a recorded value
    return [follows_instruction, FaithfulnessMetric(model=model, threshold=0.8)]


def judged_conversation_metrics(model):
    form_manners = ConversationalGEval(
        name="Form Manners",
        evaluation_steps=[
            "The assistant fills in a form with the caller, one field at a time: it asks for a value, reads "
            "it back, and goes on once the caller confirms it.",
            "Check that the assistant asks one thing at a time and never asks again for a value the caller "
            "already confirmed.",
            "Check that the assistant never says the booking is made or done before every value is confirmed.",
            "Check that every value the assistant reads back is the one the caller gave.",
        ],
        evaluation_params=[MultiTurnParams.ROLE, MultiTurnParams.CONTENT],
        model=model,
        threshold=0.7,
    )

    return [form_manners, KnowledgeRetentionMetric(model=model, threshold=0.7)]
