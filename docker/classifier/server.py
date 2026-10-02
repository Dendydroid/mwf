"""Machine #1 without an LLM: one GLiNER2 model scores the intent labels and finds the form values.

The backend says which labels and fields to look for, and decides what the scores mean for the
call (app/src/domain/intent_classifier.rs). This only runs the models.
"""
import os
import threading

import dateparser
from fastapi import FastAPI
from gliner2 import AutoExtractor
from lingua import IsoCode639_1, LanguageDetectorBuilder
from pydantic import BaseModel

MODEL = os.environ.get("CLASSIFIER_MODEL") or "fastino/gliner2.5-multi-v1"
LANGUAGES = os.environ.get("CLASSIFIER_LANGUAGES") or "en,de,ru,uk"
# Form values found with less than this are not returned
FIELD_THRESHOLD = 0.3

model = AutoExtractor.from_pretrained(MODEL)
# One turn at a time: the model is not known to be safe to share between threads
model_lock = threading.Lock()
detector = LanguageDetectorBuilder.from_iso_codes_639_1(
    *(getattr(IsoCode639_1, code.strip().upper()) for code in LANGUAGES.split(","))
).build()

app = FastAPI()


class ExtractRequest(BaseModel):
    text: str
    # label -> what the caller does
    labels: dict[str, str]
    # form field -> its description
    fields: dict[str, str] = {}


@app.get("/health")
def health():
    return {"model": MODEL}


@app.post("/extract")
def extract(request: ExtractRequest):
    schema = model.create_schema().classification("intent", request.labels, multi_label=True, cls_threshold=0.0)

    with model_lock:
        scores = model.extract(request.text, schema, include_confidence=True)["intent"]
        found = (
            model.extract_entities(request.text, request.fields, threshold=FIELD_THRESHOLD, include_confidence=True)
            if request.fields
            else {"entities": {}}
        )

    return {
        "language": language_of(request.text),
        "labels": sorted(
            ({"label": score["label"], "score": score["confidence"]} for score in scores),
            key=lambda score: -score["score"],
        ),
        "entities": [
            {"field": field, "text": value["text"], "score": value["confidence"], "date": as_date(value["text"])}
            for field, values in found["entities"].items()
            for value in values
        ],
    }


def language_of(text):
    best = detector.compute_language_confidence_values(text)[0]
    if best.value == 0:
        return None

    return {"code": best.language.iso_code_639_1.name.lower(), "confidence": best.value}


def as_date(text):
    """YYYY-MM-DD when the text names a day, in any language dateparser knows."""
    parsed = dateparser.parse(text, settings={"PREFER_DATES_FROM": "past", "REQUIRE_PARTS": ["day", "month"]})

    return parsed.date().isoformat() if parsed else None
