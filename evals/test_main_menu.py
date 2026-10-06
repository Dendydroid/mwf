"""
    The main menu: what a caller can ask for when no form is in progress. Every golden is the first
    utterance of a new call, unless `after` lists what the caller said before it.
"""
import pytest

from assistant import Call
from suite import check_turn, golden

WEATHER = "get_information[get_current_weather_in_berlin]"
RATE = "get_information[get_current_uah_per_eur]"
CALENDAR = "get_information[calendar_help]"
DOCTOR = "start_form[doctor_appointment]"
LAST_FORM = "get_information[last_filled_out_form_information]"

DEGREES = [["Grad", "°", "degree"]]
HRYVNIA = [["hryw", "hriw", "griw", "gryw", "hryv", "griv", "UAH"], ["Euro", "EUR"]]
STARTED = ["Started the doctor_appointment form.", "Next, ask the caller for: Full name of the patient."]
# The reply to an unsupported request lists what the assistant does offer
OFFER_DE = [["Wetter"], ["kurs", "währung", "hryw", "griw", "UAH", "Euro"], ["Termin", "Arzt"]]
OFFER_EN = [["weather"], ["rate", "currency", "hryv", "UAH", "euro"], ["appointment", "doctor"]]
HUMAN_DE = [["verbind", "weiterleit", "leite", "Mitarbeiter", "Kolleg", "Agent", "Mensch", "Person", "durchstell"]]
HUMAN_EN = [["transfer", "connect", "agent", "human", "representative", "operator", "person", "someone"]]

GOLDENS = [
    # ── German ─────────────────────────────────────────────────────────────────────────────────────
    golden("weather-de-1", "Wie ist das Wetter in Berlin?", lang="de", capability="weather",
           intent=WEATHER, context=["Current weather in Berlin"], says=DEGREES),
    golden("weather-de-2", "Regnet es gerade in Berlin?", lang="de", capability="weather",
           intent=WEATHER, context=["Current weather in Berlin"]),
    golden("weather-de-3", "Wie warm ist es heute in Berlin?", lang="de", capability="weather",
           intent=WEATHER, context=["Current weather in Berlin"], says=DEGREES),

    golden("rate-de-1", "Wie steht der Euro zur Hrywnja?", lang="de", capability="exchange rate",
           intent=RATE, context=["1 EUR ="], says=HRYVNIA),
    golden("rate-de-2", "Wie viel Hrywnja bekomme ich für einen Euro?", lang="de", capability="exchange rate",
           intent=RATE, context=["1 EUR ="], says=HRYVNIA),
    golden("rate-de-3", "Was ist der aktuelle Wechselkurs von Euro zu ukrainischer Hrywnja?", lang="de",
           capability="exchange rate", intent=RATE, context=["1 EUR ="], says=HRYVNIA),

    golden("start-form-de-1", "Ich möchte einen Arzttermin vereinbaren.", lang="de", capability="start a form",
           intent=DOCTOR, context=STARTED, form={"patient_name": ["queued", None]}),
    golden("start-form-de-2", "Ich brauche einen Termin beim Arzt.", lang="de", capability="start a form",
           intent=DOCTOR, context=STARTED, form={"patient_name": ["queued", None]}),
    # Symptoms alone mean a doctor's appointment too
    golden("start-form-de-symptoms-1", "Ich habe seit drei Tagen starke Kopfschmerzen.", lang="de",
           capability="start a form", intent=DOCTOR, context=STARTED, form={"patient_name": ["queued", None]}),
    golden("start-form-de-symptoms-2", "Mein Knie tut weh, ich muss zum Arzt.", lang="de",
           capability="start a form", intent=DOCTOR, context=STARTED, form={"patient_name": ["queued", None]}),

    golden("calendar-de-1", "Welches Datum ist nächsten Samstag?", lang="de", capability="calendar refusal",
           intent=CALENDAR, context=["cannot help with"]),
    golden("calendar-de-2", "Der wievielte ist übermorgen?", lang="de", capability="calendar refusal",
           intent=CALENDAR, context=["cannot help with"]),

    golden("end-call-de-1", "Danke, das war alles. Auf Wiederhören.", lang="de", capability="end call",
           intent="end_call", action="end_call", context=["The caller is ending the call"], asks="nothing"),
    golden("end-call-de-2", "Tschüss!", lang="de", capability="end call",
           intent="end_call", action="end_call", context=["The caller is ending the call"], asks="nothing"),

    golden("transfer-de-1", "Ich möchte mit einem Mitarbeiter sprechen.", lang="de", capability="transfer to a human",
           intent="transfer_to_human", action="transfer_to_human", asks="nothing", says=HUMAN_DE),
    golden("transfer-de-2", "Können Sie mich bitte mit einem Menschen verbinden?", lang="de",
           capability="transfer to a human", intent="transfer_to_human", action="transfer_to_human",
           asks="nothing", says=HUMAN_DE),

    # Close to a supported request, but not the same: another city, another currency, another booking
    golden("unsupported-de-city", "Wie ist das Wetter in München?", lang="de", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_DE),
    golden("unsupported-de-currency", "Wie steht der Dollar zum Euro?", lang="de", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_DE),
    golden("unsupported-de-booking", "Ich möchte einen Tisch im Restaurant reservieren.", lang="de",
           capability="unsupported request", intent="unsupported", context=["not supported"], says=OFFER_DE),
    golden("unsupported-de-greeting", "Hallo, wie geht es Ihnen?", lang="de", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_DE),
    golden("unsupported-de-joke", "Erzählen Sie mir bitte einen Witz.", lang="de", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_DE),

    golden("repeat-de", "Wie bitte? Können Sie das wiederholen?", after=["Wie ist das Wetter in Berlin?"],
           lang="de", capability="repeat", intent="repeat", context=["asked to hear your last reply"], says=DEGREES),
    # With nothing said yet there is nothing to repeat, and the caller hears what is offered
    golden("repeat-de-nothing-said", "Können Sie das bitte wiederholen?", lang="de", capability="repeat",
           intent=["repeat", "unsupported"], context=["not supported"], says=OFFER_DE),

    # Asked about a booking before any form was filled out, the caller hears that there is none
    golden("last-form-de-none", "Was habe ich gebucht?", lang="de", capability="last filled out form",
           intent=LAST_FORM, context=["none was filled out yet"]),

    # ── English ────────────────────────────────────────────────────────────────────────────────────
    golden("weather-en-1", "What's the weather like in Berlin?", lang="en", capability="weather",
           intent=WEATHER, context=["Current weather in Berlin"], says=DEGREES),
    golden("weather-en-2", "Is it raining in Berlin right now?", lang="en", capability="weather",
           intent=WEATHER, context=["Current weather in Berlin"]),

    golden("rate-en-1", "What is the euro to hryvnia exchange rate?", lang="en", capability="exchange rate",
           intent=RATE, context=["1 EUR ="], says=HRYVNIA),
    golden("rate-en-2", "How many hryvnias do I get for one euro?", lang="en", capability="exchange rate",
           intent=RATE, context=["1 EUR ="], says=HRYVNIA),

    golden("start-form-en-1", "I'd like to book a doctor's appointment.", lang="en", capability="start a form",
           intent=DOCTOR, context=STARTED, form={"patient_name": ["queued", None]}),
    golden("start-form-en-symptoms", "I have had a bad headache for three days.", lang="en",
           capability="start a form", intent=DOCTOR, context=STARTED, form={"patient_name": ["queued", None]}),

    golden("calendar-en", "What date is next Saturday?", lang="en", capability="calendar refusal",
           intent=CALENDAR, context=["cannot help with"]),

    golden("end-call-en", "Thanks, that's all. Goodbye.", lang="en", capability="end call",
           intent="end_call", action="end_call", context=["The caller is ending the call"], asks="nothing"),

    golden("transfer-en", "Can I talk to a real person, please?", lang="en", capability="transfer to a human",
           intent="transfer_to_human", action="transfer_to_human", asks="nothing", says=HUMAN_EN),

    golden("unsupported-en-city", "What's the weather like in Paris?", lang="en", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_EN),
    golden("unsupported-en-currency", "What is the dollar to euro rate?", lang="en", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_EN),
    golden("unsupported-en-booking", "I want to book a table at a restaurant.", lang="en",
           capability="unsupported request", intent="unsupported", context=["not supported"], says=OFFER_EN),
    golden("unsupported-en-greeting", "Hi, how are you?", lang="en", capability="unsupported request",
           intent="unsupported", context=["not supported"], says=OFFER_EN),

    golden("repeat-en", "Sorry, could you repeat that?", after=["What's the weather like in Berlin?"],
           lang="en", capability="repeat", intent="repeat", context=["asked to hear your last reply"], says=DEGREES),

    golden("last-form-en-none", "What did I book?", lang="en", capability="last filled out form",
           intent=LAST_FORM, context=["none was filled out yet"]),
]


@pytest.mark.parametrize("golden", GOLDENS, ids=lambda golden: golden.name)
def test_main_menu(golden):
    call = Call()
    for utterance in golden.additional_metadata.get("after", []):
        call.say(utterance)

    check_turn("main menu", golden, call.say(golden.input))
