"""
    Whole calls, played line by line. Every line has what its turn is expected to come to, so a call that
    goes wrong shows the turn it went wrong at. A caller who follows a script does not react to a wrong
    reply, so the turns after a wrong one show what the call's state made of it.
"""
import pytest

from suite import ANY, check_conversation, conversation, has, phrase, refusal

PROVIDE = "provide_form_field_value"
CORRECT = "correct_form_field_value"
DOCTOR = "start_form[doctor_appointment]"
FORM_QUESTION = "get_information[form_information]"
WEATHER = "get_information[get_current_weather_in_berlin]"
# What the validator of the date of birth tells a German caller
FUTURE = [refusal("date_of_birth_in_the_future", "de")]

GOLDENS = [
    # ── German ─────────────────────────────────────────────────────────────────────────────────────
    conversation(
        "booking-full-sentences-de", "A caller books a doctor's appointment and answers in full sentences.",
        lang="de", capability="book an appointment", steps=[
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR, form={"patient_name": ["queued", None]})),
            ("Der Patient heißt Hans Müller.", dict(intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Ja, das ist richtig.", dict(intent="confirm_yes", form={"patient_name": ["completed", "Hans Müller"]})),
            ("Er ist am 13. Juni 1991 geboren.", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
            ("Ja, das stimmt.", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "1991-06-13"]})),
            ("Er hat starke Kopfschmerzen.", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("kopfschmerzen")]})),
            ("Ja, genau.", dict(intent="confirm_yes", form={"reason": ["completed", ANY]})),
            ("Am liebsten nächsten Samstag.", dict(intent=PROVIDE, form={"appointment_date": ["awaiting_confirmation", has(("saturday", "samstag"))]})),
            ("Ja, das passt.", dict(intent="confirm_yes", form={"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]})),
            ("Am liebsten am Vormittag.", dict(intent=PROVIDE, form={"appointment_time": ["awaiting_confirmation", has(("morning", "vormittag"))]})),
            ("Ja, das ist gut so.", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "Hans Müller", "date_of_birth": "1991-06-13", "reason": has("kopfschmerzen"),
                           "appointment_date": has(("saturday", "samstag")),
                           "appointment_time": has(("morning", "vormittag"))})),
            ("Nein danke, auf Wiederhören.", dict(intent="end_call", action="end_call", asks="nothing")),
        ]),

    conversation(
        "booking-short-answers-de", "A caller books a doctor's appointment with the short answers a form gets.",
        lang="de", capability="book an appointment", steps=[
            ("Ich brauche einen Arzttermin.", dict(intent=DOCTOR, form={"patient_name": ["queued", None]})),
            ("Hans Müller", dict(intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Ja", dict(intent="confirm_yes", form={"patient_name": ["completed", "Hans Müller"]})),
            ("13. Juni 1991", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
            ("Ja", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "1991-06-13"]})),
            ("Kopfschmerzen", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("kopfschmerzen")]})),
            ("Ja", dict(intent="confirm_yes", form={"reason": ["completed", ANY]})),
            ("Nächsten Samstag", dict(intent=PROVIDE, form={"appointment_date": ["awaiting_confirmation", has(("saturday", "samstag"))]})),
            ("Ja", dict(intent="confirm_yes", form={"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]})),
            ("Vormittags", dict(intent=PROVIDE, form={"appointment_time": ["awaiting_confirmation", ANY]})),
            ("Ja", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "Hans Müller", "date_of_birth": "1991-06-13", "reason": has("kopfschmerzen"),
                           "appointment_date": has(("saturday", "samstag")), "appointment_time": ANY})),
        ]),

    conversation(
        "booking-with-hints-de", "The caller says who and when before the form starts, so the form only asks whether that is right.",
        lang="de", capability="book an appointment", steps=[
            ("Ich möchte einen Arzttermin für meinen Sohn Lukas Schneider, morgen um 15 Uhr.", dict(
                intent=DOCTOR, form={"patient_name": ["awaiting_confirmation", "Lukas Schneider"],
                                     "appointment_date": ["awaiting_confirmation", has("morgen")],
                                     "appointment_time": ["awaiting_confirmation", has("15")]})),
            ("Ja, das ist richtig.", dict(intent="confirm_yes", form={"patient_name": ["completed", "Lukas Schneider"]})),
            ("Er ist am 4. März 2015 geboren.", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "2015-03-04"]})),
            ("Ja, das stimmt.", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "2015-03-04"]})),
            ("Er hat Fieber und Husten.", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("fieber")]})),
            # The date and the time given at the start are read back now, one after the other, not asked for
            ("Ja, genau.", dict(intent="confirm_yes", form={"reason": ["completed", ANY],
                                                           "appointment_date": ["awaiting_confirmation", has("morgen")]})),
            ("Ja, das passt.", dict(intent="confirm_yes", form={"appointment_date": ["completed", has("morgen")],
                                                               "appointment_time": ["awaiting_confirmation", has("15")]})),
            ("Ja, das ist gut so.", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "Lukas Schneider", "date_of_birth": "2015-03-04", "reason": has("fieber"),
                           "appointment_date": has("morgen"), "appointment_time": has("15")})),
        ]),

    conversation(
        "booking-with-corrections-de", "The caller corrects a name that was read back and first gives a date of birth in the future.",
        lang="de", capability="book an appointment", steps=[
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR)),
            ("Der Patient heißt Hans Müller.", dict(intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Nein, er heißt Hans Möller.", dict(intent=CORRECT, form={"patient_name": ["awaiting_confirmation", "Hans Möller"]})),
            ("Ja, das ist richtig.", dict(intent="confirm_yes", form={"patient_name": ["completed", "Hans Möller"]})),
            ("Er ist am 13. Juni 2091 geboren.", dict(says=FUTURE, form={"date_of_birth": ["queued", None]})),
            ("Entschuldigung, er ist am 13. Juni 1991 geboren.", dict(form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
            ("Ja, das stimmt.", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "1991-06-13"]})),
            ("Er hat Rückenschmerzen.", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("rückenschmerzen")]})),
            ("Ja, genau.", dict(intent="confirm_yes", form={"reason": ["completed", ANY]})),
            # The date and the time in one answer: both are recorded, and each is confirmed with a yes of its own
            ("Übermorgen um neun Uhr.", dict(intent=PROVIDE, form={
                "appointment_date": ["awaiting_confirmation", has("übermorgen")],
                "appointment_time": ["awaiting_confirmation", has(("9", "neun", "nine"))]})),
            ("Ja, das passt.", dict(intent="confirm_yes", form={
                "appointment_date": ["completed", ANY],
                "appointment_time": ["awaiting_confirmation", has(("9", "neun", "nine"))]})),
            ("Ja, das ist gut so.", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "Hans Möller", "date_of_birth": "1991-06-13", "reason": has("rückenschmerzen"),
                           "appointment_date": ANY, "appointment_time": has(("9", "neun", "nine"))})),
        ]),

    conversation(
        "three-rejections-de", "The same value is read back wrong for the caller three times, so a human is offered.",
        lang="de", capability="offer a human", steps=[
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR)),
            ("Der Patient heißt Hans Müller.", dict(form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Nein, das ist falsch.", dict(form={"patient_name": ["queued", None]})),
            ("Der Patient heißt Hans Müller.", dict(form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Nein, das stimmt nicht.", dict(form={"patient_name": ["queued", None]})),
            ("Der Patient heißt Hans Müller.", dict(form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Nein, das ist wieder falsch.", dict(
                form={"patient_name": ["queued", None]}, says=[phrase("offer_human", "de")], asks="any")),
            ("Ja, bitte verbinden Sie mich mit einem Mitarbeiter.", dict(intent="transfer_to_human", action="transfer_to_human")),
        ]),

    conversation(
        "interruptions-de", "Inside the form the caller asks for the weather, asks about the form, has a reply repeated and then cancels.",
        lang="de", capability="interrupt a form", steps=[
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR)),
            ("Wie ist das Wetter in Berlin?", dict(intent="unsupported", form="unchanged", never_says=[["Grad", "°"]])),
            ("Der Patient heißt Hans Müller.", dict(intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Welchen Namen haben Sie notiert?", dict(intent=FORM_QUESTION, form="unchanged", says=[["Hans"], ["Müller"]])),
            ("Wie bitte? Können Sie das wiederholen?", dict(intent="repeat", form="unchanged")),
            ("Ja, das ist richtig.", dict(intent="confirm_yes", form={"patient_name": ["completed", "Hans Müller"]})),
            ("Ich möchte das abbrechen.", dict(intent="cancel_form", form="none", asks="anything_else")),
            # Back in the main menu the weather is answered again
            ("Wie ist das Wetter in Berlin?", dict(intent=WEATHER, form="none", says=[["Grad", "°"]])),
        ]),

    conversation(
        "question-is-not-a-yes-de", "When a value is read back the caller first asks whether the appointment is confirmed or written down right. No question confirms a value, and the one about the last value does not send the form.",
        lang="de", capability="question about the form", steps=[
            ("Ich möchte einen Arzttermin für meinen Sohn Lukas Schneider, morgen um 15 Uhr.", dict(
                intent=DOCTOR, form={"patient_name": ["awaiting_confirmation", "Lukas Schneider"]})),
            ("Ist der Termin damit schon bestätigt?", dict(intent=FORM_QUESTION, form="unchanged")),
            ("Ja, das ist richtig.", dict(intent="confirm_yes", form={"patient_name": ["completed", "Lukas Schneider"]})),
            ("Er ist am 4. März 2015 geboren.", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "2015-03-04"]})),
            ("Haben Sie das richtig notiert?", dict(intent=FORM_QUESTION, form="unchanged")),
            ("Ja, das stimmt.", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "2015-03-04"]})),
            ("Er hat Fieber und Husten.", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("fieber")]})),
            ("Ja, genau.", dict(intent="confirm_yes", form={"reason": ["completed", ANY],
                                                           "appointment_date": ["awaiting_confirmation", has("morgen")]})),
            ("Ja, das passt.", dict(intent="confirm_yes", form={"appointment_date": ["completed", has("morgen")],
                                                               "appointment_time": ["awaiting_confirmation", has("15")]})),
            # The last value, asked the way a speech recognizer writes it: a yes here completes the form
            ("Ist der Termin jetzt bestätigt", dict(intent=FORM_QUESTION, form="unchanged")),
            ("Ja, das ist gut so.", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "Lukas Schneider", "date_of_birth": "2015-03-04", "reason": has("fieber"),
                           "appointment_date": has("morgen"), "appointment_time": has("15")})),
        ]),

    conversation(
        "patient-is-the-caller-de", "The caller gave their name first and then says they are the patient, without saying the name again.",
        lang="de", capability="point to a value", steps=[
            ("Guten Tag, mein Name ist Peter Schulz.", dict(hints={"caller_full_name": "Peter Schulz"})),
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR)),
            ("Der Patient bin ich selbst.", dict(
                intent=["refer_to_context_for_form_field_value", PROVIDE],
                form={"patient_name": ["awaiting_confirmation", "Peter Schulz"]})),
        ]),

    conversation(
        "second-booking-de", "After one booking the caller books another one: the form starts with the values of the first and only asks whether they are right.",
        lang="de", capability="after a completed form", steps=[
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR)),
            ("Der Patient heißt Hans Müller.", dict(form={"patient_name": ["awaiting_confirmation", "Hans Müller"]})),
            ("Ja, das ist richtig.", dict(form={"patient_name": ["completed", "Hans Müller"]})),
            ("Er ist am 13. Juni 1991 geboren.", dict(form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
            ("Ja, das stimmt.", dict(form={"date_of_birth": ["completed", "1991-06-13"]})),
            ("Er hat starke Kopfschmerzen.", dict(form={"reason": ["awaiting_confirmation", ANY]})),
            ("Ja, genau.", dict(form={"reason": ["completed", ANY]})),
            ("Am liebsten nächsten Samstag.", dict(form={"appointment_date": ["awaiting_confirmation", ANY]})),
            ("Ja, das passt.", dict(form={"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]})),
            ("Am liebsten am Vormittag.", dict(form={"appointment_time": ["awaiting_confirmation", ANY]})),
            # The form's completion callback writes its values back into the hints
            ("Ja, das ist gut so.", dict(
                form="none", asks="anything_else",
                hints={"patient_full_name": "Hans Müller", "date_of_birth_iso_8601": "1991-06-13",
                       "appointment_spoken_date": has(("saturday", "samstag")),
                       "appointment_spoken_time": has(("morning", "vormittag"))})),
            ("Ich möchte noch einen Arzttermin vereinbaren.", dict(
                intent=DOCTOR, form={"patient_name": ["awaiting_confirmation", "Hans Müller"],
                                     "date_of_birth": ["awaiting_confirmation", "1991-06-13"],
                                     "reason": ["queued", None]})),
            ("Ja, das ist richtig.", dict(
                intent="confirm_yes", form={"patient_name": ["completed", "Hans Müller"],
                                            "date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
        ]),

    # The call's language follows the caller from one utterance to the next
    conversation(
        "language-switch", "The caller starts in German, goes on in English and comes back to German.",
        lang="de", capability="follow the caller's language", steps=[
            ("Ich möchte einen Arzttermin vereinbaren.", dict(intent=DOCTOR)),
            ("The patient's name is John Smith.", dict(lang="en", intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "John Smith"]})),
            ("Yes, that is correct.", dict(lang="en", intent="confirm_yes", form={"patient_name": ["completed", "John Smith"]})),
            ("Er ist am 13. Juni 1991 geboren.", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
        ]),

    # ── English ────────────────────────────────────────────────────────────────────────────────────
    conversation(
        "booking-full-sentences-en", "A caller books a doctor's appointment and answers in full sentences.",
        lang="en", capability="book an appointment", steps=[
            ("I would like to book a doctor's appointment.", dict(intent=DOCTOR, form={"patient_name": ["queued", None]})),
            ("The patient's name is John Smith.", dict(intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "John Smith"]})),
            ("Yes, that's correct.", dict(intent="confirm_yes", form={"patient_name": ["completed", "John Smith"]})),
            ("He was born on June 13th, 1991.", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
            ("Yes, that's right.", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "1991-06-13"]})),
            ("He has a bad headache.", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("headache")]})),
            ("Yes, exactly.", dict(intent="confirm_yes", form={"reason": ["completed", ANY]})),
            ("Next Saturday, please.", dict(intent=PROVIDE, form={"appointment_date": ["awaiting_confirmation", has("saturday")]})),
            ("Yes, that works.", dict(intent="confirm_yes", form={"appointment_date": ["completed", ANY], "appointment_time": ["queued", None]})),
            ("In the morning, please.", dict(intent=PROVIDE, form={"appointment_time": ["awaiting_confirmation", has("morning")]})),
            ("Yes, that is fine.", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "John Smith", "date_of_birth": "1991-06-13", "reason": has("headache"),
                           "appointment_date": has("saturday"), "appointment_time": has("morning")})),
            ("No thanks, goodbye.", dict(intent="end_call", action="end_call", asks="nothing")),
        ]),

    conversation(
        "booking-short-answers-with-corrections-en", "A caller books with short answers and corrects two values that were read back.",
        lang="en", capability="book an appointment", steps=[
            ("I need to see a doctor.", dict(intent=DOCTOR, form={"patient_name": ["queued", None]})),
            ("John Smith", dict(intent=PROVIDE, form={"patient_name": ["awaiting_confirmation", "John Smith"]})),
            ("No, it's John Smyth.", dict(intent=CORRECT, form={"patient_name": ["awaiting_confirmation", "John Smyth"]})),
            ("Yes", dict(intent="confirm_yes", form={"patient_name": ["completed", "John Smyth"]})),
            ("June 13th 1991", dict(intent=PROVIDE, form={"date_of_birth": ["awaiting_confirmation", "1991-06-13"]})),
            ("Yes, but it's 1992.", dict(form={"date_of_birth": ["awaiting_confirmation", "1992-06-13"]})),
            ("Yes", dict(intent="confirm_yes", form={"date_of_birth": ["completed", "1992-06-13"]})),
            ("A sore throat", dict(intent=PROVIDE, form={"reason": ["awaiting_confirmation", has("sore throat")]})),
            ("Yes", dict(intent="confirm_yes", form={"reason": ["completed", ANY]})),
            # The date and the time in one answer: both are recorded, and each is confirmed with a yes of its own
            ("Tomorrow at 3 pm", dict(intent=PROVIDE, form={
                "appointment_date": ["awaiting_confirmation", has("tomorrow")],
                "appointment_time": ["awaiting_confirmation", has(("3", "15"))]})),
            ("Yes", dict(intent="confirm_yes", form={
                "appointment_date": ["completed", ANY],
                "appointment_time": ["awaiting_confirmation", has(("3", "15"))]})),
            ("Yes", dict(
                intent="confirm_yes", form="none", asks="anything_else",
                completed={"patient_name": "John Smyth", "date_of_birth": "1992-06-13", "reason": has("sore throat"),
                           "appointment_date": has("tomorrow"), "appointment_time": has(("3", "15"))})),
        ]),

    conversation(
        "hang-up-inside-form-en", "The caller says goodbye in the middle of the form.",
        lang="en", capability="end call", steps=[
            ("I would like to book a doctor's appointment.", dict(intent=DOCTOR)),
            ("The patient's name is John Smith.", dict(intent=PROVIDE)),
            ("Actually, never mind. Goodbye.", dict(intent="end_call", action="end_call", form="none", asks="nothing")),
        ]),
]


@pytest.mark.parametrize("golden", GOLDENS, ids=lambda golden: golden.name)
def test_conversation(golden):
    check_conversation("conversations", golden)
