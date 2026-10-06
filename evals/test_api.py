"""
    What the app does around the three machines: the HTTP contract, the call session in Redis, one turn of
    a call at a time, a turn that fails on the app's side, and what becomes of a completed form.
"""
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

from assistant import APP_LOG, ENDPOINT, HTTP, Call
from suite import check_facts, check_turn, golden

RESPONSE_KEYS = {
    "answer", "selected_function", "confidence", "language_detected", "action", "reasoning",
    "response_context", "hint_map", "form",
}
HINT_KEYS = {
    "caller_full_name", "patient_full_name", "date_of_birth_iso_8601", "appointment_spoken_date",
    "appointment_spoken_time", "last_filled_out_form",
}
SESSION_TTL_SECONDS = 3600

# Longer than the model's context window, so the first machine of the turn fails
TOO_LONG = {
    "de": "Ich möchte bitte einen Termin beim Arzt vereinbaren. " * 2500,
    "en": "I would like to book an appointment with the doctor, please. " * 2500,
}


def test_call_id_is_required():
    response = HTTP.post(ENDPOINT, json={"request_text": "Hallo"})

    check_facts("api", "call-id-is-required", "HTTP contract", "a request without x-call-id", response.text, {
        "the status is 400": response.status_code == 400,
        "the body says the header is required": "x-call-id header is required" in response.text,
    })


def test_request_text_is_required():
    response = HTTP.post(ENDPOINT, headers={"x-call-id": Call().call_id}, json={"request_text": "   "})

    check_facts("api", "request-text-is-required", "HTTP contract", "a request with only spaces as its text", response.text, {
        "the status is 400": response.status_code == 400,
        "the body says the text must not be empty": "request_text must not be empty" in response.text,
    })


def test_response_has_every_key():
    turn = Call().say("Wie ist das Wetter in Berlin?")

    check_facts("api", "response-has-every-key", "HTTP contract", turn.utterance, turn.answer, {
        "the status is 200": turn.status == 200,
        f"the body has exactly {sorted(RESPONSE_KEYS)}": set(turn.body) == RESPONSE_KEYS,
        f"hint_map has exactly {sorted(HINT_KEYS)}": set(turn.body.get("hint_map") or {}) == HINT_KEYS,
        "confidence is between 0 and 1": 0 <= (turn.body.get("confidence") or -1) <= 1,
    })


def test_pages_are_served():
    page, version = HTTP.get("/mimic-client"), HTTP.get("/version")

    check_facts("api", "pages-are-served", "HTTP contract", "GET /mimic-client and GET /version", version.text, {
        "the mimic client is an HTML page": page.status_code == 200 and "text/html" in page.headers.get("content-type", ""),
        "the version has version and environment": version.status_code == 200 and {"version", "environment"} <= set(version.json()),
    })


def test_session_is_kept_between_turns():
    call = Call()
    started = call.say("Ich möchte einen Arzttermin vereinbaren.")
    ttl_after_first = call.session_ttl()
    time.sleep(2)
    named = call.say("Der Patient heißt Hans Müller.")
    ttl_after_second = call.session_ttl()

    fresh = lambda ttl: SESSION_TTL_SECONDS - 2 <= ttl <= SESSION_TTL_SECONDS
    check_facts("api", "session-is-kept-between-turns", "call session", "two turns of one call, two seconds apart", named.answer, {
        "the first turn starts the form": started.form is not None,
        "the second turn finds the form and fills it": named.form is not None and named.intent == "provide_form_field_value",
        f"the session is saved for {SESSION_TTL_SECONDS} s": fresh(ttl_after_first),
        # A sliding expiry: a call is forgotten an hour after its last turn, not after its first
        "every turn starts the hour again": fresh(ttl_after_second),
    })


def test_turns_of_one_call_run_one_at_a_time():
    if not APP_LOG.enabled:
        pytest.skip("the order of the turns is read from the app log")

    call = Call()
    call.say("Ich möchte einen Arzttermin vereinbaren.")
    say = lambda text: HTTP.post(ENDPOINT, headers={"x-call-id": call.call_id}, json={"request_text": text})
    with ThreadPoolExecutor(max_workers=2) as pool:
        responses = list(pool.map(say, ["Der Patient heißt Hans Müller.", "Ja, das ist richtig."]))

    time.sleep(0.5)
    APP_LOG.read()
    order = [record.kind for record in APP_LOG.pending.pop(call.call_id, []) if record.kind in ("handling", "done")]

    check_facts("api", "turns-of-one-call-run-one-at-a-time", "call session", "two turns of one call sent at the same moment", str(order), {
        "both are answered": all(response.status_code == 200 for response in responses),
        "the second starts once the first is answered": order == ["handling", "done", "handling", "done"],
    })


def test_completed_form_is_handed_over(form_states):
    if not APP_LOG.enabled:
        pytest.skip("what became of the form is read from the app log")

    call, _ = form_states.at("de", "time_awaiting")
    turn = call.say("Ja, das ist gut so.")
    outcome = turn.stages.get("completed_form")

    # Without FORM_SUBMIT_URL in .env the submitter has nowhere to send it, and says so in the log. With
    # it the POST is made, and whether it arrives is the receiving project's to say
    check_facts("api", "completed-form-is-handed-over", "form submission", turn.utterance, f"{turn.answer} ({outcome})", {
        "the turn completes the form": turn.form is None and bool(turn.body["hint_map"].get("last_filled_out_form")),
        "the form reaches the submitter once the reply is formulated": outcome is not None,
    })


FAILED_TURNS = [
    golden("failed-turn-main-menu-de", "(longer than the model's context window)", lang="de",
           capability="a turn that fails", failed=True, intent=None, form="none",
           says=[["Fehler aufgetreten"], ["Wetter"], ["Arzttermin"]]),
    # Inside a form the caller only has to say it again: the form is as it was
    golden("failed-turn-inside-form-de", "(longer than the model's context window)", state="dob_awaiting",
           lang="de", capability="a turn that fails", failed=True, intent=None, form="unchanged",
           says=[["Fehler aufgetreten"], ["wiederholen"]]),
    golden("failed-turn-main-menu-en", "(longer than the model's context window)", lang="en",
           capability="a turn that fails", failed=True, intent=None, form="none",
           says=[["error happened on our side"], ["weather"], ["appointment"]]),
]


@pytest.mark.parametrize("golden", FAILED_TURNS, ids=lambda golden: golden.name)
def test_failed_turn(golden, form_states):
    about = golden.additional_metadata
    call, form_before = form_states.at(about["lang"], about["state"]) if "state" in about else (Call(), None)

    check_turn("api", golden, call.say(TOO_LONG[about["lang"]]), form_before)
