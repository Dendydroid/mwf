import deepeval
import httpx
import pytest
from deepeval.utils import get_is_running_deepeval

from assistant import APP_URL, HTTP, REDIS
from judge import JUDGE_MODEL, LLM_URL
from metrics import TURN_BUDGET_MS
from suite import FormStates


def pytest_sessionstart(session):
    try:
        HTTP.get("/version").raise_for_status()
        REDIS.ping()
    except Exception as error:
        pytest.exit(
            f"The app at {APP_URL} or its Redis cannot be reached ({error}). "
            "Start them with `docker compose up -d` in the project root.",
            returncode=2,
        )


@pytest.fixture(scope="session")
def form_states():
    return FormStates()


def served_model():
    try:
        return httpx.get(f"{LLM_URL}/models", timeout=10).json()["data"][0]["id"]
    except Exception:
        return "unknown"


def hyperparameters():
    return {
        "app": HTTP.get("/version").json().get("version", "unknown"),
        "model": served_model(),
        "judge": JUDGE_MODEL or "none",
        "turn budget ms": TURN_BUDGET_MS,
    }


@pytest.fixture(scope="session", autouse=True)
def made_with():
    """What the run was made with, kept in its results file. DeepEval's test run only exists once the
    session has started, so this is not done when the file is imported."""
    if get_is_running_deepeval():
        deepeval.log_hyperparameters(hyperparameters)
