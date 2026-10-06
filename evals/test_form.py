"""
    The doctor form, one answer at a time. `state` is where in the form the caller says it (the walk in
    suite.py): a field is `queued` when the caller was asked for it, and `awaiting` when its value was
    read back and the caller was asked whether it is right. The walk gives Hans Müller, born 1991-06-13,
    with a headache, for next Saturday, in the morning (John Smith in English).
"""
import pytest

from suite import ANY, check_turn, confirm, form_says, golden, has, phrase, refusal

PROVIDE = "provide_form_field_value"
CORRECT = "correct_form_field_value"
FORM_QUESTION = "get_information[form_information]"
WEATHER = "get_information[get_current_weather_in_berlin]"
DOCTOR = "start_form[doctor_appointment]"
LAST_FORM = "get_information[last_filled_out_form_information]"

LANGUAGES = ("de", "en")
# What code says as it is, word for word as the vocabulary has it, by the caller's language
CANNOT_HELP = {lang: [phrase("cannot_help_in_form", lang)] for lang in LANGUAGES}
FUTURE = {lang: [refusal("date_of_birth_in_the_future", lang)] for lang in LANGUAGES}
SORRY = {lang: [phrase("sorry", lang)] for lang in LANGUAGES}
THANKS = {lang: [phrase("thanks", lang)] for lang in LANGUAGES}
COMPLETED = {lang: [form_says("completed", lang)] for lang in LANGUAGES}
CANCELLED = {lang: [form_says("cancelled", lang)] for lang in LANGUAGES}
# What the main menu's formulator is told about the form that was filled out last
SUMMARIZE = ["Summarize it", "patient_name:"]
# A second form in the call: what the first one wrote back into the hints waits to be confirmed again
SECOND_FORM = {
    "patient_name": ["awaiting_confirmation", "Hans Müller"],
    "date_of_birth": ["awaiting_confirmation", "1991-06-13"],
    "reason": ["queued", None],
    "appointment_date": ["awaiting_confirmation", ANY],
    "appointment_time": ["awaiting_confirmation", ANY],
}

GOLDENS = [
    # ── German: asked for the patient's name ───────────────────────────────────────────────────────
    golden("name-bare-de", "Hans Müller", state="name_queued", lang="de", capability="give a value",
           intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "Hans Müller"]}),
    golden("name-sentence-de", "Der Patient heißt Hans Müller.", state="name_queued", lang="de",
           capability="give a value", intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "Hans Müller"]}),
    golden("form-question-what-is-needed-de", "Welche Angaben brauchen Sie von mir?", state="name_queued",
           lang="de", capability="question about the form", intent=FORM_QUESTION, form="unchanged",
           context=["The caller asked about the form"]),
    # Weather and the rate are main menu requests: inside a form they are turned down and the step's question is asked again
    golden("weather-inside-form-de", "Wie ist das Wetter in Berlin?", state="name_queued", lang="de",
           capability="outside request in a form", intent="unsupported", form="unchanged", says=CANNOT_HELP["de"],
           never_says=[["Grad", "°"]]),
    golden("cancel-de", "Ich möchte das abbrechen.", state="name_queued", lang="de", capability="cancel the form",
           intent="cancel_form", form="none", says=CANCELLED["de"], asks="anything_else"),
    golden("transfer-inside-form-de", "Ich möchte lieber mit einem Menschen sprechen.", state="name_queued",
           lang="de", capability="transfer to a human", intent="transfer_to_human", action="transfer_to_human",
           form="unchanged"),
    golden("end-call-inside-form-de", "Ich muss jetzt auflegen, auf Wiederhören.", state="name_queued", lang="de",
           capability="end call", intent="end_call", action="end_call", form="none"),
    # A yes with nothing to confirm changes nothing
    golden("yes-with-nothing-to-confirm-de", "Ja", state="name_queued", lang="de",
           capability="confirm or reject", form="unchanged"),
    # Several values in one answer are all recorded. Each then waits for its own yes, in the form's order
    golden("several-values-de",
           "Der Patient heißt Hans Müller, er hat starke Kopfschmerzen und möchte morgen um 15 Uhr kommen.",
           state="name_queued", lang="de", capability="several values in one answer", intent=PROVIDE,
           form={"patient_name": ["awaiting_confirmation", "Hans Müller"], "date_of_birth": ["queued", None],
                 "reason": ["awaiting_confirmation", has("kopfschmerzen")],
                 "appointment_date": ["awaiting_confirmation", has(("tomorrow", "morgen"))],
                 "appointment_time": ["awaiting_confirmation", has(("15", "3"))]}),

    # ── German: the name was read back ─────────────────────────────────────────────────────────────
    golden("name-yes-de", "Ja", state="name_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_yes", says=THANKS["de"],
           form={"patient_name": ["completed", "Hans Müller"], "date_of_birth": ["queued", None]}),
    golden("name-yes-sentence-de", "Ja, das ist richtig.", state="name_awaiting", lang="de",
           capability="confirm or reject", intent="confirm_yes",
           form={"patient_name": ["completed", "Hans Müller"], "date_of_birth": ["queued", None]}),
    golden("name-yes-genau-de", "Genau.", state="name_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_yes", form={"patient_name": ["completed", "Hans Müller"]}),
    golden("name-no-de", "Nein", state="name_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_no", says=SORRY["de"], form={"patient_name": ["queued", None]}),
    golden("name-no-sentence-de", "Nein, das stimmt nicht.", state="name_awaiting", lang="de",
           capability="confirm or reject", intent="confirm_no", says=SORRY["de"],
           form={"patient_name": ["queued", None]}),
    golden("name-no-with-the-right-one-de", "Nein, er heißt Hans Möller.", state="name_awaiting", lang="de",
           capability="correct a value", intent=CORRECT, form={"patient_name": ["awaiting_confirmation", "Hans Möller"]}),
    # Only a part is corrected: the rest of the value stays
    golden("name-yes-but-de", "Ja, aber der Nachname ist Meier.", state="name_awaiting", lang="de",
           capability="correct a value", intent=[CORRECT, "confirm_yes"],
           form={"patient_name": ["awaiting_confirmation", "Hans Meier"]}),
    # Answering the next question accepts the value that was read back
    golden("name-moving-on-de", "Er ist am 13. Juni 1991 geboren.", state="name_awaiting", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"patient_name": ["completed", "Hans Müller"], "date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    # Values for later fields alone accept the value that was read back, and none of them is accepted with it
    golden("several-values-moving-on-de", "Er ist am 13. Juni 1991 geboren und hat Rückenschmerzen.",
           state="name_awaiting", lang="de", capability="several values in one answer", intent=PROVIDE,
           form={"patient_name": ["completed", "Hans Müller"], "date_of_birth": ["awaiting_confirmation", "1991-06-13"],
                 "reason": ["awaiting_confirmation", has("rückenschmerzen")]},
           known_gap="values for two later fields are taken for a yes to the value read back, and are lost (gap 10)"),
    golden("unclear-word-de", "Hmm", state="name_awaiting", lang="de", capability="confirm or reject",
           intent="unsupported", form="unchanged",
           known_gap="unclear words are taken for a yes (gap 7)"),
    golden("repeat-inside-form-de", "Wie bitte? Können Sie das wiederholen?", state="name_awaiting", lang="de",
           capability="repeat", intent="repeat", form="unchanged", says=[confirm("patient_name", "Hans Müller", "de")]),
    golden("form-question-recorded-name-de", "Welchen Namen haben Sie notiert?", state="name_awaiting", lang="de",
           capability="question about the form", intent=FORM_QUESTION, form="unchanged",
           says=[["Hans"], ["Müller"]]),
    # A question is never a yes, also when it asks whether something is confirmed or right: the value that
    # was read back stays as it is and the question is answered
    golden("form-question-confirmed-de", "Ist der Termin damit schon bestätigt?", state="name_awaiting", lang="de",
           capability="question about the form", intent=FORM_QUESTION, form="unchanged",
           context=["The caller asked about the form"]),
    golden("form-question-written-down-right-de", "Haben Sie das richtig notiert?", state="name_awaiting", lang="de",
           capability="question about the form", intent=FORM_QUESTION, form="unchanged"),
    # An agreement in the words of that question is still one
    golden("name-yes-confirming-de", "Ja, das kann ich bestätigen.", state="name_awaiting", lang="de",
           capability="confirm or reject", intent="confirm_yes", says=THANKS["de"],
           form={"patient_name": ["completed", "Hans Müller"], "date_of_birth": ["queued", None]}),

    # ── German: asked for the date of birth ────────────────────────────────────────────────────────
    golden("birth-sentence-de", "Er ist am 13. Juni 1991 geboren.", state="dob_queued", lang="de",
           capability="give a value", intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    golden("birth-bare-de", "13. Juni 1991", state="dob_queued", lang="de", capability="give a value",
           intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    golden("birth-digits-de", "13.06.1991", state="dob_queued", lang="de", capability="give a value",
           intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    # The way a speech recognizer writes a spoken date
    golden("birth-in-words-de", "Am dreizehnten Juni neunzehnhunderteinundneunzig.", state="dob_queued",
           lang="de", capability="give a value", intent=PROVIDE,
           form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    golden("birth-in-the-future-de", "Er ist am 13. Juni 2091 geboren.", state="dob_queued", lang="de",
           capability="validator", intent=PROVIDE, says=FUTURE["de"], form={"date_of_birth": ["queued", None]}),
    golden("birth-unreadable-de", "Irgendwann im Sommer.", state="dob_queued", lang="de",
           capability="give a value", form="unchanged",
           known_gap="an answer the asked field cannot take is recorded for a later field it fits (gap 10)"),
    # A field that is already confirmed is opened again
    golden("correct-confirmed-name-de", "Moment, der Name ist falsch, der Patient heißt Hans Meier.",
           state="dob_queued", lang="de", capability="correct a value", intent=CORRECT,
           form={"patient_name": ["awaiting_confirmation", "Hans Meier"]}),
    golden("rate-inside-form-de", "Wie steht der Euro zur Hrywnja?", state="dob_queued", lang="de",
           capability="outside request in a form", intent="unsupported", form="unchanged", says=CANNOT_HELP["de"]),

    # ── German: the date of birth 1991-06-13 was read back ─────────────────────────────────────────
    golden("birth-yes-de", "Ja", state="dob_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_yes", form={"date_of_birth": ["completed", "1991-06-13"], "reason": ["queued", None]}),
    golden("birth-yes-stimmt-de", "Stimmt.", state="dob_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_yes", form={"date_of_birth": ["completed", "1991-06-13"]}),
    golden("birth-no-de", "Nein", state="dob_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_no", says=SORRY["de"], form={"date_of_birth": ["queued", None]}),
    golden("birth-no-other-year-de", "Nein, 1992.", state="dob_awaiting", lang="de", capability="correct a value",
           intent=CORRECT, form={"date_of_birth": ["awaiting_confirmation", "1992-06-13"]}),
    golden("birth-yes-but-other-year-de", "Ja, aber 1992.", state="dob_awaiting", lang="de",
           capability="correct a value", intent=[CORRECT, "confirm_yes"],
           form={"date_of_birth": ["awaiting_confirmation", "1992-06-13"]}),
    golden("birth-no-other-day-de", "Nein, am vierzehnten.", state="dob_awaiting", lang="de",
           capability="correct a value", intent=CORRECT, form={"date_of_birth": ["awaiting_confirmation", "1991-06-14"]}),
    # A refused correction still says the value that was read back is wrong
    golden("birth-no-future-year-de", "Nein, er ist 2092 geboren.", state="dob_awaiting", lang="de",
           capability="validator", says=FUTURE["de"], form={"date_of_birth": ["queued", None]}),
    golden("birth-moving-on-de", "Er hat Rückenschmerzen.", state="dob_awaiting", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"date_of_birth": ["completed", "1991-06-13"],
                 "reason": ["awaiting_confirmation", has("rückenschmerzen")]}),
    # The way a speech recognizer writes a question: without its question mark
    golden("form-question-confirmed-as-recognized-de", "Ist der Termin jetzt bestätigt", state="dob_awaiting",
           lang="de", capability="question about the form", intent=FORM_QUESTION, form="unchanged"),

    # ── German: the reason for the visit ───────────────────────────────────────────────────────────
    golden("reason-bare-de", "Kopfschmerzen", state="reason_queued", lang="de", capability="give a value",
           intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("kopfschmerzen")]}),
    golden("reason-sentence-de", "Er hat seit drei Tagen starke Rückenschmerzen.", state="reason_queued",
           lang="de", capability="give a value", intent=PROVIDE,
           form={"reason": ["awaiting_confirmation", has("rückenschmerzen")]}),
    golden("form-question-booked-yet-de", "Haben Sie den Termin schon gebucht?", state="reason_queued", lang="de",
           capability="question about the form", intent=FORM_QUESTION, form="unchanged"),
    golden("several-values-later-fields-de", "Er hat Rückenschmerzen und kann nächsten Samstag am Vormittag.",
           state="reason_queued", lang="de", capability="several values in one answer", intent=PROVIDE,
           form={"reason": ["awaiting_confirmation", has("rückenschmerzen")],
                 "appointment_date": ["awaiting_confirmation", has(("saturday", "samstag"))],
                 "appointment_time": ["awaiting_confirmation", has(("morning", "vormittag"))]}),
    golden("reason-yes-de", "Ja", state="reason_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_yes", form={"reason": ["completed", ANY], "appointment_date": ["queued", None]}),
    golden("reason-no-with-the-right-one-de", "Nein, eigentlich sind es Rückenschmerzen.", state="reason_awaiting",
           lang="de", capability="correct a value", intent=CORRECT,
           form={"reason": ["awaiting_confirmation", has("rückenschmerzen")]}),

    # ── German: the date of the appointment, kept as the caller said it ────────────────────────────
    golden("appointment-weekday-de", "Am nächsten Samstag.", state="date_queued", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has(("saturday", "samstag"))],
                 "appointment_time": ["queued", None]}),
    golden("appointment-tomorrow-de", "Morgen, wenn es geht.", state="date_queued", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has(("tomorrow", "morgen"))]}),
    golden("appointment-date-de", "Am 20. Oktober.", state="date_queued", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has(("20", "twentieth"), ("october", "oktober"))]}),
    # The date and the time are two steps: said in one answer, both are recorded and confirmed one after the other
    golden("appointment-date-with-time-de", "Morgen um 15 Uhr.", state="date_queued", lang="de",
           capability="date and time in one answer", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has(("tomorrow", "morgen"))],
                 "appointment_time": ["awaiting_confirmation", has(("15", "3"))]}),
    golden("appointment-weekday-with-time-de", "Nächsten Samstag vormittags.", state="date_queued", lang="de",
           capability="date and time in one answer", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has(("saturday", "samstag"))],
                 "appointment_time": ["awaiting_confirmation", has(("morning", "vormittag", "noon"))]}),
    # A calendar question is not an appointment date
    golden("calendar-inside-form-de", "Welches Datum ist nächsten Samstag?", state="date_queued", lang="de",
           capability="outside request in a form", intent="unsupported", form="unchanged", says=CANNOT_HELP["de"]),
    golden("appointment-date-yes-de", "Ja", state="date_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_yes", form={"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]}),
    golden("appointment-date-no-with-the-right-one-de", "Nein, lieber am Sonntag.", state="date_awaiting", lang="de",
           capability="correct a value", intent=CORRECT,
           form={"appointment_date": ["awaiting_confirmation", has(("sunday", "sonntag"))]}),
    golden("appointment-date-no-de", "Nein", state="date_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_no", says=SORRY["de"], form={"appointment_date": ["queued", None]}),
    # Answering with the time accepts the date that was read back
    golden("appointment-date-moving-on-de", "Um 15 Uhr.", state="date_awaiting", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_date": ["completed", ANY], "appointment_time": ["awaiting_confirmation", has(("15", "3"))]}),

    # ── German: the time of the appointment ────────────────────────────────────────────────────────
    golden("appointment-time-clock-de", "Um 15 Uhr.", state="time_queued", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_time": ["awaiting_confirmation", has(("15", "3"))]}),
    golden("appointment-time-of-day-de", "Am Vormittag.", state="time_queued", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_time": ["awaiting_confirmation", has(("morning", "vormittag"))]}),
    golden("appointment-time-sentence-de", "Am besten um halb elf.", state="time_queued", lang="de",
           capability="give a value", intent=PROVIDE,
           form={"appointment_time": ["awaiting_confirmation", has(("10", "halb elf", "half past ten"))]}),
    golden("appointment-yes-completes-de", "Ja", state="time_awaiting", lang="de", capability="complete the form",
           intent="confirm_yes", form="none", says=COMPLETED["de"],
           completed={"patient_name": "Hans Müller", "date_of_birth": "1991-06-13",
                      "reason": has("kopfschmerzen"), "appointment_date": has(("saturday", "samstag")),
                      "appointment_time": has(("morning", "vormittag"))},
           asks="anything_else"),
    golden("appointment-time-no-with-the-right-one-de", "Nein, lieber am Nachmittag.", state="time_awaiting",
           lang="de", capability="correct a value", intent=CORRECT,
           form={"appointment_time": ["awaiting_confirmation", has(("afternoon", "nachmittag"))]}),
    golden("appointment-time-no-de", "Nein", state="time_awaiting", lang="de", capability="confirm or reject",
           intent="confirm_no", says=SORRY["de"], form={"appointment_time": ["queued", None]}),
    # Asked about the last value, a question taken for a yes would complete the form and send it
    golden("form-question-confirmed-last-value-de", "Ist der Termin damit schon bestätigt?", state="time_awaiting",
           lang="de", capability="question about the form", intent=FORM_QUESTION, form="unchanged"),

    # ── German: back in the main menu with a completed form ────────────────────────────────────────
    # The form's completion callback wrote its values back into the hints, where they stay
    golden("weather-after-form-de", "Wie ist das Wetter in Berlin?", state="completed", lang="de",
           capability="after a completed form", intent=WEATHER, form="none",
           hints={"caller_full_name": None, "patient_full_name": "Hans Müller", "date_of_birth_iso_8601": "1991-06-13",
                  "appointment_spoken_date": has(("saturday", "samstag")),
                  "appointment_spoken_time": has(("morning", "vormittag"))}),
    golden("second-form-starts-with-the-last-values-de", "Ich möchte noch einen Arzttermin vereinbaren.",
           state="completed", lang="de", capability="after a completed form", intent=DOCTOR, form=SECOND_FORM),
    golden("question-about-booking-de", "Für wen habe ich den Termin gebucht?", state="completed", lang="de",
           capability="after a completed form", intent=LAST_FORM, form="none", context=SUMMARIZE,
           says=[["Hans"], ["Müller"]]),
    # What was booked is the appointment: the day of the walk has to be in the summary
    golden("question-what-was-booked-de", "Was habe ich gebucht?", state="completed", lang="de",
           capability="after a completed form", intent=LAST_FORM, form="none", context=SUMMARIZE,
           says=[["Samstag"]]),
    golden("nothing-else-de", "Nein danke, das war alles.", state="completed", lang="de",
           capability="end call", intent="end_call", action="end_call", asks="nothing"),

    # ── English ────────────────────────────────────────────────────────────────────────────────────
    golden("name-bare-en", "John Smith", state="name_queued", lang="en", capability="give a value",
           intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "John Smith"]}),
    golden("weather-inside-form-en", "What's the weather like in Berlin?", state="name_queued", lang="en",
           capability="outside request in a form", intent="unsupported", form="unchanged", says=CANNOT_HELP["en"],
           never_says=[["degree", "°"]]),
    golden("cancel-en", "I want to cancel this.", state="name_queued", lang="en", capability="cancel the form",
           intent="cancel_form", form="none", says=CANCELLED["en"], asks="anything_else"),
    golden("several-values-en", "John Smith, he has a bad headache and would like to come tomorrow at 3 pm.",
           state="name_queued", lang="en", capability="several values in one answer", intent=PROVIDE,
           form={"patient_name": ["awaiting_confirmation", "John Smith"], "date_of_birth": ["queued", None],
                 "reason": ["awaiting_confirmation", has("headache")],
                 "appointment_date": ["awaiting_confirmation", has("tomorrow")],
                 "appointment_time": ["awaiting_confirmation", has(("3", "15"))]}),

    golden("name-yes-en", "Yes", state="name_awaiting", lang="en", capability="confirm or reject",
           intent="confirm_yes", form={"patient_name": ["completed", "John Smith"], "date_of_birth": ["queued", None]}),
    golden("name-no-en", "No", state="name_awaiting", lang="en", capability="confirm or reject",
           intent="confirm_no", says=SORRY["en"], form={"patient_name": ["queued", None]}),
    golden("name-no-with-the-right-one-en", "No, it's John Smyth.", state="name_awaiting", lang="en",
           capability="correct a value", intent=CORRECT, form={"patient_name": ["awaiting_confirmation", "John Smyth"]}),
    golden("name-moving-on-en", "He was born on June 13th, 1991.", state="name_awaiting", lang="en",
           capability="give a value", intent=PROVIDE,
           form={"patient_name": ["completed", "John Smith"], "date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    golden("unclear-word-en", "hmm", state="name_awaiting", lang="en", capability="confirm or reject",
           intent="unsupported", form="unchanged",
           known_gap="unclear words are taken for a yes (gap 7)"),
    golden("form-question-confirmed-en", "Is the appointment confirmed now?", state="name_awaiting", lang="en",
           capability="question about the form", intent=FORM_QUESTION, form="unchanged",
           context=["The caller asked about the form"]),
    golden("name-yes-confirming-en", "Yes, I can confirm that.", state="name_awaiting", lang="en",
           capability="confirm or reject", intent="confirm_yes", says=THANKS["en"],
           form={"patient_name": ["completed", "John Smith"], "date_of_birth": ["queued", None]}),

    golden("birth-bare-en", "June 13th 1991", state="dob_queued", lang="en", capability="give a value",
           intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]}),
    golden("birth-in-the-future-en", "He was born on June 13th, 2091.", state="dob_queued", lang="en",
           capability="validator", intent=PROVIDE, says=FUTURE["en"], form={"date_of_birth": ["queued", None]}),

    golden("birth-yes-but-other-year-en", "Yes, but it's 1992.", state="dob_awaiting", lang="en",
           capability="correct a value", intent=[CORRECT, "confirm_yes"],
           form={"date_of_birth": ["awaiting_confirmation", "1992-06-13"]}),
    golden("birth-no-other-day-en", "No, the fourteenth.", state="dob_awaiting", lang="en",
           capability="correct a value", intent=CORRECT, form={"date_of_birth": ["awaiting_confirmation", "1991-06-14"]}),
    golden("birth-no-future-year-en", "No, he was born in 2092.", state="dob_awaiting", lang="en",
           capability="validator", says=FUTURE["en"], form={"date_of_birth": ["queued", None]}),
    golden("form-question-got-that-right-en", "Did you get that right?", state="dob_awaiting", lang="en",
           capability="question about the form", intent=FORM_QUESTION, form="unchanged"),

    golden("reason-sentence-en", "He has had a sore throat for three days.", state="reason_queued", lang="en",
           capability="give a value", intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("sore throat")]}),
    golden("appointment-weekday-en", "Next Saturday.", state="date_queued", lang="en",
           capability="give a value", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has("saturday")], "appointment_time": ["queued", None]}),
    golden("appointment-date-with-time-en", "Tomorrow at 3 pm.", state="date_queued", lang="en",
           capability="date and time in one answer", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has("tomorrow")],
                 "appointment_time": ["awaiting_confirmation", has(("3", "15"))]}),
    golden("appointment-weekday-with-time-en", "Next Saturday in the morning.", state="date_queued", lang="en",
           capability="date and time in one answer", intent=PROVIDE,
           form={"appointment_date": ["awaiting_confirmation", has("saturday")],
                 "appointment_time": ["awaiting_confirmation", has("morning")]}),
    golden("appointment-date-yes-en", "Yes", state="date_awaiting", lang="en", capability="confirm or reject",
           intent="confirm_yes", form={"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]}),
    golden("appointment-date-no-with-the-right-one-en", "No, Sunday would be better.", state="date_awaiting",
           lang="en", capability="correct a value", intent=CORRECT,
           form={"appointment_date": ["awaiting_confirmation", has("sunday")]}),
    golden("appointment-date-moving-on-en", "At 3 pm.", state="date_awaiting", lang="en",
           capability="give a value", intent=PROVIDE,
           form={"appointment_date": ["completed", ANY], "appointment_time": ["awaiting_confirmation", has(("3", "15"))]}),
    golden("appointment-time-clock-en", "At 3 pm.", state="time_queued", lang="en",
           capability="give a value", intent=PROVIDE,
           form={"appointment_time": ["awaiting_confirmation", has(("3", "15"))]}),
    golden("appointment-time-of-day-en", "In the afternoon.", state="time_queued", lang="en",
           capability="give a value", intent=PROVIDE,
           form={"appointment_time": ["awaiting_confirmation", has("afternoon")]}),
    golden("appointment-yes-completes-en", "Yes", state="time_awaiting", lang="en",
           capability="complete the form", intent="confirm_yes", form="none",
           says=COMPLETED["en"],
           completed={"patient_name": "John Smith", "date_of_birth": "1991-06-13",
                      "reason": has("headache"), "appointment_date": has("saturday"),
                      "appointment_time": has("morning")},
           asks="anything_else"),
    golden("appointment-time-no-with-the-right-one-en", "No, the afternoon would be better.", state="time_awaiting",
           lang="en", capability="correct a value", intent=CORRECT,
           form={"appointment_time": ["awaiting_confirmation", has("afternoon")]}),
    golden("form-question-confirmed-last-value-en", "Is the appointment confirmed now?", state="time_awaiting",
           lang="en", capability="question about the form", intent=FORM_QUESTION, form="unchanged"),

    golden("question-about-booking-en", "What did I book?", state="completed", lang="en",
           capability="after a completed form", intent=LAST_FORM, form="none", context=SUMMARIZE,
           says=[["Saturday"]]),
]


@pytest.mark.parametrize("golden", GOLDENS, ids=lambda golden: golden.name)
def test_form(golden, form_states):
    about = golden.additional_metadata
    call, form_before = form_states.at(about["lang"], about["state"])

    check_turn("form", golden, call.say(golden.input), form_before)
