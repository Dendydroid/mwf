"""
    Hints: what the caller already says in the main menu for a form that is started later. The hints are
    kept for the call, and a started form is filled from them, each value waiting for the caller's yes.
"""
import pytest

from assistant import Call
from suite import NO_HINTS, ask, check_turn, confirm, golden, has

DOCTOR = "start_form[doctor_appointment]"
ASK_NAME = {lang: [ask("patient_name", lang)] for lang in ("de", "en")}

GOLDENS = [
    # ── German ─────────────────────────────────────────────────────────────────────────────────────
    golden("patient-name-de", "Ich möchte einen Arzttermin für meinen Sohn Lukas Schneider vereinbaren.",
           lang="de", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Lukas Schneider", "date_of_birth_iso_8601": None},
           form={"patient_name": ["awaiting_confirmation", "Lukas Schneider"], "date_of_birth": ["queued", None]},
           says=[confirm("patient_name", "Lukas Schneider", "de")]),

    golden("date-and-time-de", "Ich brauche einen Arzttermin für morgen um 15 Uhr.",
           lang="de", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": None, "appointment_spoken_date": has("morgen"),
                  "appointment_spoken_time": has("15")},
           form={"patient_name": ["queued", None], "appointment_date": ["awaiting_confirmation", has("morgen")],
                 "appointment_time": ["awaiting_confirmation", has("15")]},
           says=ASK_NAME["de"]),

    golden("name-and-birth-de", "Meine Tochter Mia Wagner, geboren am 4. März 2015, braucht einen Termin beim Arzt.",
           lang="de", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Mia Wagner", "date_of_birth_iso_8601": "2015-03-04"},
           form={"patient_name": ["awaiting_confirmation", "Mia Wagner"],
                 "date_of_birth": ["awaiting_confirmation", "2015-03-04"]}),

    golden("everything-at-once-de",
           "Ich möchte für meinen Sohn Lukas Schneider, geboren am 4. März 2015, einen Arzttermin am nächsten Montag um 10 Uhr.",
           lang="de", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Lukas Schneider", "date_of_birth_iso_8601": "2015-03-04",
                  "appointment_spoken_date": has("montag"), "appointment_spoken_time": has("10")},
           form={"patient_name": ["awaiting_confirmation", "Lukas Schneider"],
                 "date_of_birth": ["awaiting_confirmation", "2015-03-04"],
                 "reason": ["queued", None],
                 "appointment_date": ["awaiting_confirmation", has("montag")],
                 "appointment_time": ["awaiting_confirmation", has("10")]}),

    # A hint the field's validator refuses is left out without a word, and the field is asked for
    golden("birth-in-the-future-de", "Ich brauche einen Arzttermin für Max Braun, geboren am 13. Juni 2091.",
           lang="de", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Max Braun", "date_of_birth_iso_8601": "2091-06-13"},
           form={"patient_name": ["awaiting_confirmation", "Max Braun"], "date_of_birth": ["queued", None]}),

    # The caller's own name is not the patient's: the form still asks who the patient is
    golden("callers-own-name-de", "Guten Tag, hier ist Anna Becker. Ich brauche einen Arzttermin.",
           lang="de", capability="hints fill the form", intent=DOCTOR,
           hints={"caller_full_name": "Anna Becker", "patient_full_name": None},
           form={"patient_name": ["queued", None]}, says=ASK_NAME["de"]),

    # Hints are kept for the call: a later turn adds to them and takes none away
    golden("kept-over-turns-de", "Ich möchte einen Arzttermin für meine Frau Sabine Schulz vereinbaren.",
           after=["Guten Tag, mein Name ist Peter Schulz."],
           lang="de", capability="hints are kept", intent=DOCTOR,
           hints={"caller_full_name": "Peter Schulz", "patient_full_name": "Sabine Schulz"},
           form={"patient_name": ["awaiting_confirmation", "Sabine Schulz"]}),

    golden("nothing-to-take-de", "Wie ist das Wetter in Berlin?",
           lang="de", capability="hints are kept", hints=NO_HINTS),

    # ── English ────────────────────────────────────────────────────────────────────────────────────
    golden("patient-name-en", "I'd like to book a doctor's appointment for my son Lucas Miller.",
           lang="en", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Lucas Miller", "date_of_birth_iso_8601": None},
           form={"patient_name": ["awaiting_confirmation", "Lucas Miller"], "date_of_birth": ["queued", None]},
           says=[confirm("patient_name", "Lucas Miller", "en")]),

    golden("date-and-time-en", "I need a doctor's appointment for tomorrow at 3 pm.",
           lang="en", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": None, "appointment_spoken_date": has("tomorrow"),
                  "appointment_spoken_time": has(("3", "15"))},
           form={"patient_name": ["queued", None],
                 "appointment_date": ["awaiting_confirmation", has("tomorrow")],
                 "appointment_time": ["awaiting_confirmation", has(("3", "15"))]},
           says=ASK_NAME["en"]),

    golden("name-and-birth-en", "My daughter Emma Clark, born on March 4th 2015, needs to see a doctor.",
           lang="en", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Emma Clark", "date_of_birth_iso_8601": "2015-03-04"},
           form={"patient_name": ["awaiting_confirmation", "Emma Clark"],
                 "date_of_birth": ["awaiting_confirmation", "2015-03-04"]}),

    golden("birth-in-the-future-en", "I need a doctor's appointment for Max Brown, born on June 13th 2091.",
           lang="en", capability="hints fill the form", intent=DOCTOR,
           hints={"patient_full_name": "Max Brown", "date_of_birth_iso_8601": "2091-06-13"},
           form={"patient_name": ["awaiting_confirmation", "Max Brown"], "date_of_birth": ["queued", None]}),

    golden("callers-own-name-en", "Hello, my name is John Smith. I need a doctor appointment.",
           lang="en", capability="hints fill the form", intent=DOCTOR,
           hints={"caller_full_name": "John Smith", "patient_full_name": None},
           form={"patient_name": ["queued", None]}, says=ASK_NAME["en"]),

    golden("nothing-to-take-en", "What's the weather like in Berlin?",
           lang="en", capability="hints are kept", hints=NO_HINTS),
]


@pytest.mark.parametrize("golden", GOLDENS, ids=lambda golden: golden.name)
def test_hints(golden):
    call = Call()
    for utterance in golden.additional_metadata.get("after", []):
        call.say(utterance)

    check_turn("hints", golden, call.say(golden.input))
