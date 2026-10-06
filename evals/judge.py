"""
    The model that grades a reply for the LLM-judged metrics. Without EVAL_JUDGE_MODEL there is none, and
    the suites run their own checks only.

    EVAL_JUDGE_MODEL=gpt-4.1         a model DeepEval knows by name, with OPENAI_API_KEY set
    EVAL_JUDGE_MODEL=claude-...      an Anthropic model, with ANTHROPIC_API_KEY set
    EVAL_JUDGE_MODEL=local           the project's own vLLM. The 7B model is a poor judge: on replies
                                     labelled by hand it agreed in about two of three, see the README
"""
import os

import httpx
from deepeval.models import AnthropicModel, DeepEvalBaseLLM
from openai import AsyncOpenAI, OpenAI

LLM_URL = os.getenv("EVAL_LLM_URL", "http://localhost:8000/v1")
LLM_API_KEY = os.getenv("EVAL_LLM_API_KEY", "none")
JUDGE_MODEL = os.getenv("EVAL_JUDGE_MODEL", "")
TIMEOUT_SECONDS = 120


class VllmJudge(DeepEvalBaseLLM):
    """Holds every verdict to the JSON schema the metric asks for, the way the app holds its machines
    to theirs, so the small model cannot write a verdict that does not parse."""

    def __init__(self):
        self.served_model = httpx.get(f"{LLM_URL}/models", timeout=10).json()["data"][0]["id"]
        self.client = OpenAI(base_url=LLM_URL, api_key=LLM_API_KEY, timeout=TIMEOUT_SECONDS)
        self.async_client = AsyncOpenAI(base_url=LLM_URL, api_key=LLM_API_KEY, timeout=TIMEOUT_SECONDS)

        super().__init__(self.served_model)

    def load_model(self):
        return self.client

    def get_model_name(self):
        return f"{self.served_model} (vLLM)"

    def generate(self, prompt, schema=None):
        response = self.client.chat.completions.create(**self.request(prompt, schema))

        return self.read(response, schema)

    async def a_generate(self, prompt, schema=None):
        response = await self.async_client.chat.completions.create(**self.request(prompt, schema))

        return self.read(response, schema)

    def request(self, prompt, schema):
        request = {
            "model": self.served_model,
            "temperature": 0.0,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": prompt}],
        }
        if schema is not None:
            request["response_format"] = {
                "type": "json_schema",
                "json_schema": {"name": schema.__name__, "schema": schema.model_json_schema(), "strict": True},
            }

        return request

    def read(self, response, schema):
        content = response.choices[0].message.content

        return content if schema is None else schema.model_validate_json(content)


def judge():
    """What a judged metric gets as its `model`, or None when no judge is configured."""
    if not JUDGE_MODEL:
        return None
    if JUDGE_MODEL == "local":
        return VllmJudge()
    if JUDGE_MODEL.startswith("claude"):
        return AnthropicModel(model=JUDGE_MODEL)

    return JUDGE_MODEL
