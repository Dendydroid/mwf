# Prompts

`system_prompt.txt` is the instruction the model gets for every utterance. The
whole file is sent verbatim as the `system` message, trailing newline trimmed.

It is a file rather than a Rust constant so that trying a different prompt is an
edit, not a rebuild - and it is **re-read on every request**, so an edit takes
effect on the next thing you say. Nothing to restart. (`docker-compose.yml`
mounts this directory into the container for the same reason.)

Which file is read comes from `LLM_SYSTEM_PROMPT_FILE` in `.env`, so several
prompts can live here side by side and be swapped by pointing that at another
one - that needs a restart, because the path is read once at startup.

## What the rest of the system requires of it

Exactly one thing: the JSON schema must keep asking for
`humanlike_sentence_answer`. That is the sentence read back to the caller, so
the code has to know its name; drop it and every turn fails with a clear
`missing field` error rather than answering silence.

Every *other* key is free. Add one and it travels the whole way on its own - the
endpoint passes it through under its own name, and the mimic client renders a
badge for it, typed by its value: a bool lights up or dims, a 0..1 number gets a
percentage and a meter, anything else is shown as itself. Keys appear in the
order this schema lists them.

Two things worth knowing:

- Every key costs generated tokens, and `llm_settings.max_tokens` in
  `config/settings.toml` is the ceiling (256). The eight keys here come to about
  100. Past the cap a turn fails with a message naming that setting, rather than
  a confusing parse error.
- Write the schema as real JSON, keys quoted. Unquoted keys happen to work -
  the model quotes them anyway - but it means asking a model that is constrained
  to emit strict JSON to copy something that is not.

## If the file is broken

A read failure (mid-save, deleted, renamed) is logged as a warning and the last
prompt that loaded successfully keeps being used, so a slip of the editor cannot
take the assistant down. An empty file is refused the same way.
