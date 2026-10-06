"""
    Talks to the running app the way the phone side does: one utterance per HTTP request, tied to a call
    by `x-call-id`. Also reads what a turn left behind in Redis and in the app's log file.
"""
import os
import re
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

import httpx
import redis

APP_URL = os.getenv("EVAL_APP_URL", "http://localhost:8080").rstrip("/")
REDIS_URL = os.getenv("EVAL_REDIS_URL", "redis://localhost:6379")
LOG_DIR = Path(os.getenv("EVAL_APP_LOG_DIR", Path(__file__).resolve().parent.parent / "logs"))

ENDPOINT = "/assistant/handle-request"
# A copied session is kept this long, so a long run does not lose the states it probes from
SNAPSHOT_TTL_SECONDS = 6 * 3600

HTTP = httpx.Client(base_url=APP_URL, timeout=180)
REDIS = redis.Redis.from_url(REDIS_URL)


def session_key(call_id):
    return f"call_id.{call_id}"


@dataclass
class Turn:
    """One utterance and everything the app answered to it."""

    call_id: str
    utterance: str
    status: int
    body: dict
    # As the phone side sees it
    latency_ms: float
    # Where the time went inside the app, from its log. Empty when the log cannot be read
    stages: dict = field(default_factory=dict)

    @property
    def answer(self):
        return self.body.get("answer") or ""

    @property
    def intent(self):
        return self.body.get("selected_function")

    @property
    def form(self):
        return self.body.get("form")

    @property
    def response_context(self):
        return self.body.get("response_context")


class Call:
    def __init__(self, call_id=None):
        self.call_id = call_id or f"eval-{uuid.uuid4().hex[:12]}"
        self.turns = []

    def say(self, utterance):
        started = time.perf_counter()
        response = HTTP.post(ENDPOINT, headers={"x-call-id": self.call_id}, json={"request_text": utterance})
        latency_ms = (time.perf_counter() - started) * 1000

        is_json = response.headers.get("content-type", "").startswith("application/json")
        turn = Turn(
            call_id=self.call_id,
            utterance=utterance,
            status=response.status_code,
            body=response.json() if is_json else {"error": response.text},
            latency_ms=round(latency_ms, 1),
            stages=timed(APP_LOG.stages(self.call_id), latency_ms) if response.status_code == 200 else {},
        )
        self.turns.append(turn)

        return turn

    def copy(self):
        """The same call under another id, as Redis has it now: for trying several answers at one
        point of a call without playing the call again for each."""
        other = Call()
        REDIS.copy(session_key(self.call_id), session_key(other.call_id))
        REDIS.expire(session_key(other.call_id), SNAPSHOT_TTL_SECONDS)

        return other

    def session_ttl(self):
        return REDIS.ttl(session_key(self.call_id))


# --------------------------------------------------------------------------------------------------
#  The app log: `Machine prompt` and `Machine answered` of every machine, with the call id
# --------------------------------------------------------------------------------------------------

RECORD_START = re.compile(r"(?m)^(?=\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+Z\s)")
CALL_ID = re.compile(r'call_id="([^"]*)"')
PAYLOAD_CALL_ID = re.compile(r'"call_id":"([^"]*)"')
OUTPUT = re.compile(r'output="(\w+)"')
CONTENT = re.compile(r'content="(.*)"\s*$', re.S)

MACHINES = {
    "HintMap": "context_extractor",
    "ExtractedFormValues": "context_extractor",
    "ExtractedMainMenuIntent": "intent_matcher",
    "ExtractedFormIntent": "intent_matcher",
    # Asked after the form's matcher when it chose `confirm_yes`: is the utterance a question?
    "CheckedAgreement": "agreement_checker",
    "FormulatedResponse": "response_formulator",
    # The form formulator's answer to a question about the form
    "FormulatedAnswer": "response_formulator",
}


@dataclass
class Record:
    at: datetime
    # handling, prompt, answered, done, or form_not_sent and form_sent for a completed form
    kind: str
    call_id: str
    machine: str = ""
    # The machine's output type, which says which flow's machine it is
    output: str = ""
    chars: int = 0


def parse_record(text):
    header, _, rest = text.partition(": ")
    try:
        at = datetime.strptime(header.split()[0][:26], "%Y-%m-%dT%H:%M:%S.%f")
    except (ValueError, IndexError):
        return None

    if rest.startswith("Handling assistant request"):
        kind, call_ids = "handling", CALL_ID.findall(rest)[:1]
    elif rest.startswith("Answered assistant request"):
        kind, call_ids = "done", CALL_ID.findall(rest)[:1]
    elif rest.startswith("Machine answered"):
        kind, call_ids = "answered", CALL_ID.findall(rest)[:1]
    elif rest.startswith("Machine prompt"):
        # The prompt comes first here and holds the caller's words, so the fields are the last ones
        kind, call_ids = "prompt", CALL_ID.findall(rest)[-1:]
    elif rest.startswith("FORM_SUBMIT_URL is not set"):
        kind, call_ids = "form_not_sent", CALL_ID.findall(rest)[:1]
    elif rest.startswith("Form was submitted!"):
        kind, call_ids = "form_sent", PAYLOAD_CALL_ID.findall(rest)[:1]
    else:
        return None
    if not call_ids:
        return None

    record = Record(at=at, kind=kind, call_id=call_ids[0])
    if kind in ("prompt", "answered"):
        outputs = OUTPUT.findall(rest)
        output = outputs[-1] if kind == "prompt" and outputs else (outputs[0] if outputs else "")
        record.machine, record.output = MACHINES.get(output, output), output
        content = CONTENT.search(rest) if kind == "answered" else None
        record.chars = len(content.group(1)) if content else len(rest)

    return record


class AppLog:
    """The app's rolling log file, read from where the suites started."""

    def __init__(self, directory):
        self.directory = directory
        self.enabled = directory.is_dir()
        self.path = self.today()
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        # Records of each call that no turn has taken yet
        self.pending = {}

    def today(self):
        return self.directory / f"app.log.{datetime.now(timezone.utc):%Y-%m-%d}"

    def read(self):
        paths = [self.path] if self.path == self.today() else [self.path, self.today()]
        for path in paths:
            if path != self.path:
                self.path, self.offset = path, 0
            if not path.exists():
                continue

            with open(path, "rb") as file:
                file.seek(self.offset)
                chunk = file.read()
            # Only whole lines: the rest is still being written
            chunk = chunk[: chunk.rfind(b"\n") + 1]
            self.offset += len(chunk)

            for text in RECORD_START.split(chunk.decode("utf-8", errors="replace")):
                record = parse_record(text) if text else None
                if record:
                    self.pending.setdefault(record.call_id, []).append(record)

    def records(self, call_id, wait_seconds=3.0):
        """What the app logged for the turn of `call_id` that was just answered."""
        if not self.enabled:
            return []

        deadline = time.monotonic() + wait_seconds
        while True:
            self.read()
            records = self.pending.get(call_id, [])
            if any(record.kind == "done" for record in records):
                return self.pending.pop(call_id)
            if time.monotonic() > deadline:
                break
            time.sleep(0.02)

        # Nothing of the turn got here in time: the log is not the running app's, so stop waiting for it
        self.enabled = bool(self.pending.pop(call_id, []))

        return []

    def stages(self, call_id):
        return stages_of(self.records(call_id))


def milliseconds(start, end):
    return round((end - start).total_seconds() * 1000, 1)


def stages_of(records):
    """How long each step of a turn took, from its log records, in the order the turn runs them."""
    stages = {}
    handling = next((record for record in records if record.kind == "handling"), None)
    done = next((record for record in records if record.kind == "done"), None)
    prompts = [record for record in records if record.kind == "prompt"]
    answers = {record.machine: record for record in records if record.kind == "answered"}

    if handling and prompts:
        stages["flow"] = "form" if prompts[0].output == "ExtractedFormValues" else "main menu"
        stages["language_detection_ms"] = milliseconds(handling.at, prompts[0].at)

    for prompt in prompts:
        answer = answers.get(prompt.machine)
        if answer:
            stages[f"{prompt.machine}_ms"] = milliseconds(prompt.at, answer.at)
            stages[f"{prompt.machine}_prompt_chars"] = prompt.chars
            stages[f"{prompt.machine}_answer_chars"] = answer.chars

    # Between the last answer about the intent and the formulator's prompt the intent handler runs:
    # the state change, the validators and whatever it fetches
    matched = answers.get("agreement_checker") or answers.get("intent_matcher")
    formulating = next((prompt for prompt in prompts if prompt.machine == "response_formulator"), None)
    if matched and formulating:
        stages["intent_handler_ms"] = milliseconds(matched.at, formulating.at)

    if handling and done:
        stages["server_ms"] = milliseconds(handling.at, done.at)

    # What became of a form the turn completed
    kinds = {record.kind for record in records}
    if "form_sent" in kinds:
        stages["completed_form"] = "posted to FORM_SUBMIT_URL"
    elif "form_not_sent" in kinds:
        stages["completed_form"] = "not sent, FORM_SUBMIT_URL is not set"

    return stages


def timed(stages, latency_ms):
    """The log's times are the app's wall clock. In a Docker VM that clock is set back now and then, and
    a turn it was set back in has times that do not add up: those are dropped, the rest is kept."""
    times = [value for key, value in stages.items() if key.endswith("_ms")]
    server_ms = stages.get("server_ms")
    adds_up = server_ms is not None and min(times) >= 0 and abs(server_ms - latency_ms) <= 0.2 * latency_ms + 50

    return stages if adds_up else {key: value for key, value in stages.items() if not key.endswith(("_ms", "_chars"))}


APP_LOG = AppLog(LOG_DIR)
