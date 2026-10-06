"""
    Reads the results file of a run and writes what it says about the assistant: which part of a turn
    fails, for which capability and language, where the time of a turn goes, and what to look at first.

        python report.py                                   the latest run in results/
        python report.py results/test_run_<...>.json       that run

    The results file is DeepEval's own (`DEEPEVAL_RESULTS_FOLDER`). The checks that need no judge are
    worked out again from the turns it holds, one turn at a time, so a conversation counts with every
    turn it has. The judged metrics are taken as the run scored them.
"""
import json
import os
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

os.environ.setdefault("DEEPEVAL_TELEMETRY_OPT_OUT", "1")

from metrics import CHECKS, STAGES, TURN_BUDGET_MS, run_check

RESULTS = Path(__file__).resolve().parent / "results"
LATENCY = "Turn Latency"
LANGUAGES = ["de", "en"]
# The steps of a turn in the order it runs them, as the app's log times them
STEPS = [
    ("language_detection", "language detection"),
    ("context_extractor", "context extractor"),
    ("intent_matcher", "intent matcher"),
    ("agreement_checker", "agreement checker (after a yes in a form)"),
    ("intent_handler", "intent handler (state, validators, fetch)"),
    ("response_formulator", "response formulator"),
]


def load(path):
    """Every test case of the run as {name, suite, lang, capability, known_gap, metrics, turns}, a turn
    being {expect, turn}."""
    run = json.loads(Path(path).read_text(encoding="utf-8"))
    cases = []
    for raw in run.get("testCases", []) + run.get("conversationalTestCases", []):
        about = raw.get("metadata") or {}
        expect = about.get("expect", {})
        turns = about.get("turns") or ([{"expect": expect, "turn": about["turn"]}] if "turn" in about else [])
        cases.append({
            "name": raw.get("name") or "?",
            "suite": about.get("suite", "?"),
            "lang": expect.get("lang", "any"),
            "capability": expect.get("capability", "?"),
            "known_gap": expect.get("known_gap"),
            "metrics": raw.get("metricsData") or [],
            "turns": turns,
        })

    return run, cases


def checked(case):
    """The checks of every turn of a case, worked out again: [(turn, check, score, reason)]."""
    outcomes = []
    for data in case["turns"]:
        for name in CHECKS:
            outcome = run_check(name, data["expect"], data["turn"])
            if outcome is not None:
                outcomes.append((data, name, float(outcome[0]), outcome[1]))

    return outcomes


def stored(case):
    """The metrics of a case that cannot be worked out from a turn again, as the run scored them."""
    return [metric for metric in case["metrics"] if metric["name"] not in CHECKS]


def failures(case):
    """What a case failed, the latency check aside: [(check, reason)]."""
    several = len(case["turns"]) > 1
    failed = [
        (name, f"turn {case['turns'].index(data) + 1} \"{data['turn']['utterance'][:60]}\": {reason}" if several else reason)
        for data, name, score, reason in checked(case)
        if score < 1.0 and name != LATENCY
    ]

    return failed + [(metric["name"], metric.get("reason") or metric.get("error")) for metric in stored(case) if not metric["success"]]


def passed_quality(case):
    return not failures(case)


def share(passed, total):
    return f"{passed}/{total} ({100 * passed / total:.0f}%)" if total else "–"


def percentile(values, fraction):
    ordered = sorted(values)

    return ordered[min(len(ordered) - 1, int(round(fraction * (len(ordered) - 1))))]


def table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    lines += ["| " + " | ".join(str(cell).replace("|", "/").replace("\n", " ") for cell in row) + " |" for row in rows]

    return "\n".join(lines)


def languages_of(cases):
    present = {case["lang"] for case in cases}

    return [lang for lang in LANGUAGES if lang in present] + sorted(present - set(LANGUAGES))


# --------------------------------------------------------------------------------------------------
#  The parts of the report
# --------------------------------------------------------------------------------------------------


def by_check(cases):
    """Per check and language: the turns it passed, of the turns it applied to."""
    counts = defaultdict(lambda: [0, 0, 0])
    for case in cases:
        for data, name, score, _ in checked(case):
            entry = counts[(name, data["expect"]["lang"])]
            entry[0] += score >= 1.0
            entry[1] += 1
            entry[2] += score < 1.0 and bool(case["known_gap"])
        # Judged metrics and the ones about a whole case
        for metric in stored(case):
            entry = counts[(metric["name"], case["lang"])]
            entry[0] += bool(metric["success"])
            entry[1] += 1
            entry[2] += (not metric["success"]) and bool(case["known_gap"])

    return counts


def check_section(cases):
    counts = by_check(cases)
    languages = sorted({lang for _, lang in counts}, key=lambda lang: (lang not in LANGUAGES, lang))
    names = [name for name in STAGES if any((name, lang) in counts for lang in languages)]
    names += sorted({name for name, _ in counts} - set(names))

    rows = []
    for name in names:
        cells = [share(*counts[(name, lang)][:2]) if (name, lang) in counts else "–" for lang in languages]
        known = sum(counts[(name, lang)][2] for lang in languages if (name, lang) in counts)
        rows.append([name, STAGES.get(name, "response formulator (judged)"), *cells, known or ""])

    return table(["Check", "Part of the turn", *languages, "failures that are known gaps"], rows)


def capability_section(cases):
    languages = languages_of(cases)
    counts = defaultdict(lambda: [0, 0])
    for case in cases:
        if case["known_gap"]:
            continue
        entry = counts[(case["suite"], case["capability"], case["lang"])]
        entry[0] += passed_quality(case)
        entry[1] += 1

    keys = sorted({(suite, capability) for suite, capability, _ in counts})
    rows = [
        [suite, capability, *[share(*counts[(suite, capability, lang)]) if (suite, capability, lang) in counts else "–" for lang in languages]]
        for suite, capability in keys
    ]
    totals = [share(*map(sum, zip(*[counts[key] for key in counts if key[2] == lang]))) if any(key[2] == lang for key in counts) else "–" for lang in languages]

    return table(["Suite", "Capability", *languages], rows + [["**all**", "", *totals]])


def first_failures(cases):
    """Per turn that failed a check: the first check it failed, which is the part of the turn to look at."""
    counts = Counter()
    for case in cases:
        if case["known_gap"]:
            continue
        failed_turns = defaultdict(list)
        for data, name, score, _ in checked(case):
            if score < 1.0 and name != LATENCY:
                failed_turns[id(data)].append((name, data["expect"]["lang"]))
        for failures in failed_turns.values():
            counts[failures[0]] += 1

    return counts


def first_failure_section(cases):
    counts = first_failures(cases)
    languages = sorted({lang for _, lang in counts}, key=lambda lang: (lang not in LANGUAGES, lang))
    names = [name for name in STAGES if any((name, lang) in counts for lang in languages)]
    rows = [[name, STAGES[name], *[counts.get((name, lang), 0) or "" for lang in languages]] for name in names]

    return table(["First check a turn failed", "Part of the turn", *languages], rows) if rows else "No turn failed a check."


def all_turns(cases):
    return [data["turn"] for case in cases for data in case["turns"]]


def timing_section(cases):
    turns = [turn for turn in all_turns(cases) if turn.get("stages", {}).get("server_ms")]
    if not turns:
        return "The app log was not read during this run, so there are no timings of the steps of a turn."

    parts = []
    for flow in ("main menu", "form"):
        in_flow = [turn for turn in turns if turn["stages"].get("flow") == flow]
        if not in_flow:
            continue

        whole = [turn["stages"]["server_ms"] for turn in in_flow]
        rows = []
        for key, label in STEPS:
            times = [turn["stages"][f"{key}_ms"] for turn in in_flow if f"{key}_ms" in turn["stages"]]
            if not times:
                continue
            prompts = [turn["stages"][f"{key}_prompt_chars"] for turn in in_flow if f"{key}_prompt_chars" in turn["stages"]]
            answers = [turn["stages"][f"{key}_answer_chars"] for turn in in_flow if f"{key}_answer_chars" in turn["stages"]]
            rows.append([
                label,
                f"{statistics.median(times):.0f}",
                f"{percentile(times, 0.95):.0f}",
                f"{max(times):.0f}",
                f"{100 * sum(times) / sum(whole):.0f}%",
                f"{statistics.median(prompts):.0f}" if prompts else "",
                f"{statistics.median(answers):.0f}" if answers else "",
            ])
        rows.append(["**whole turn, inside the app**", f"{statistics.median(whole):.0f}", f"{percentile(whole, 0.95):.0f}", f"{max(whole):.0f}", "100%", "", ""])
        heard = [turn["latency_ms"] for turn in in_flow]
        rows.append(["whole turn, as the caller waits", f"{statistics.median(heard):.0f}", f"{percentile(heard, 0.95):.0f}", f"{max(heard):.0f}", "", "", ""])

        parts.append(f"**{flow.capitalize()} turns ({len(in_flow)})**\n\n" + table(
            ["Step", "median ms", "p95 ms", "max ms", "share of the time", "prompt, characters", "answer, characters"], rows
        ))

    untimed = len(all_turns(cases)) - len(turns)
    parts.append(
        "The steps are timed by the app's clock and the caller's wait by this machine's. In a Docker VM the two "
        f"do not run alike, so the steps can add up to a little more than the wait. {untimed} turns are left out "
        "here: the app's clock was set back during them, or they failed before a machine was asked."
    )

    everything = all_turns(cases)
    slow = sorted(everything, key=lambda turn: -turn["latency_ms"])[:5]
    over = [turn for turn in everything if turn["latency_ms"] > TURN_BUDGET_MS]
    slowest_step = lambda stages: max((stages.get(f"{key}_ms", 0), label) for key, label in STEPS)[1] if "server_ms" in stages else ""
    parts.append(
        f"**Slowest turns** ({len(over)} of {len(everything)} over the budget of {TURN_BUDGET_MS:.0f} ms)\n\n"
        + table(["ms", "Flow", "Intent", "Slowest step", "Utterance"], [
            [f"{turn['latency_ms']:.0f}", turn["stages"].get("flow", ""), turn.get("selected_function"),
             slowest_step(turn["stages"]), turn["utterance"][:60]]
            for turn in slow
        ])
    )

    return "\n\n".join(parts)


def confidence_section(cases):
    """Whether the matcher's confidence tells its wrong intents from its right ones."""
    right, wrong = [], []
    for case in cases:
        for data in case["turns"]:
            outcome = CHECKS["Intent"](data["expect"], data["turn"], "")
            confidence = data["turn"].get("confidence")
            if outcome is not None and confidence is not None:
                (right if outcome[0] else wrong).append((confidence, data["turn"]))

    if not right or not wrong:
        return f"{len(right)} intents were right and {len(wrong)} wrong, so there is nothing to compare."

    sure = sum(confidence >= 0.99 for confidence, _ in wrong)
    lowest_right = min(confidence for confidence, _ in right)
    caught = sum(confidence < lowest_right for confidence, _ in wrong)
    rows = [
        ["right", len(right), f"{statistics.median(c for c, _ in right):.4f}", f"{lowest_right:.4f}", f"{max(c for c, _ in right):.4f}"],
        ["wrong", len(wrong), f"{statistics.median(c for c, _ in wrong):.4f}", f"{min(c for c, _ in wrong):.4f}", f"{max(c for c, _ in wrong):.4f}"],
    ]

    return (
        table(["Intent", "turns", "median confidence", "lowest", "highest"], rows)
        + f"\n\n{sure} of the {len(wrong)} wrong intents were matched with a confidence of 0.99 or more. "
        f"A cut-off just under the lowest right one ({lowest_right:.4f}) would catch {caught} of them."
    )


def known_gap_section(cases):
    groups = defaultdict(list)
    for case in cases:
        if case["known_gap"]:
            groups[(case["known_gap"], case["name"])].append(passed_quality(case))
    if not groups:
        return "No golden of a known gap was run."

    rows = [
        [gap, name, share(sum(passes), len(passes)), "still open" if sum(passes) < len(passes) else "passes now"]
        for (gap, name), passes in sorted(groups.items())
    ]

    return table(["Known gap", "Golden", "passed", ""], rows)


def app_failures(cases):
    """Turns the app answered with its apology although the golden did not make them fail."""
    return [
        data["turn"] for case in cases for data in case["turns"]
        if data["turn"].get("selected_function") is None and not data["expect"].get("failed")
    ]


def failure_section(cases):
    """Every golden that failed a check and is not a known gap, with what the caller said and heard."""
    groups = defaultdict(list)
    for case in cases:
        if not case["known_gap"]:
            groups[(case["suite"], case["name"])].append(case)

    blocks = []
    for (suite, name), runs in sorted(groups.items()):
        failed = [case for case in runs if not passed_quality(case)]
        if not failed:
            continue

        # The first run that failed stands for the others
        case = failed[0]
        lines = [f"**{suite} / {name}** ({case['lang']}, {case['capability']}), failed in {len(failed)} of {len(runs)} runs"]
        if len(case["turns"]) == 1:
            turn = case["turns"][0]["turn"]
            lines.append(f"- caller: {turn['utterance'][:160]}")
            lines.append(f"- matched: `{turn.get('selected_function')}`, told: {turn.get('response_context')}")
            lines.append(f"- reply: {turn.get('answer')}")
        for check, reason in failures(case):
            lines.append(f"- ✗ {check}: {reason}")
        blocks.append("\n".join(lines))

    return "\n\n".join(blocks) if blocks else "No golden failed outside the known gaps."


def summary(run, cases):
    """What to look at first, worked out from the numbers above."""
    lines = []
    regular = [case for case in cases if not case["known_gap"]]
    for lang in languages_of(regular):
        of_lang = [case for case in regular if case["lang"] == lang]
        lines.append(f"- **{lang}**: {share(sum(map(passed_quality, of_lang)), len(of_lang))} of the goldens outside the known gaps passed every check.")

    counts = by_check(cases)
    weakest = sorted(
        ((passed / total, name, lang, passed, total) for (name, lang), (passed, total, _) in counts.items() if total >= 5 and name != LATENCY and passed < total),
    )[:5]
    for _, name, lang, passed, total in weakest:
        lines.append(f"- Weakest check: **{name}** in {lang}, passed {share(passed, total)} ({STAGES.get(name, 'judged')}).")

    first = first_failures(cases).most_common(3)
    if first:
        lines.append("- A failing turn most often fails first at: " + ", ".join(f"{name} in {lang} ({count} turns)" for (name, lang), count in first) + ".")

    heard = [turn["latency_ms"] for turn in all_turns(cases)]
    if heard:
        over = sum(latency > TURN_BUDGET_MS for latency in heard)
        lines.append(
            f"- A turn takes {statistics.median(heard):.0f} ms at the median and {percentile(heard, 0.95):.0f} ms at p95; "
            f"{over} of {len(heard)} turns were over the budget of {TURN_BUDGET_MS:.0f} ms."
        )

    turns = [turn for turn in all_turns(cases) if turn.get("stages", {}).get("server_ms")]
    if turns:
        totals = {label: sum(turn["stages"].get(f"{key}_ms", 0) for turn in turns) for key, label in STEPS}
        slowest = max(totals, key=totals.get)
        whole = sum(turn["stages"]["server_ms"] for turn in turns)
        lines.append(f"- The step that takes most of a turn is the **{slowest}**, {100 * totals[slowest] / whole:.0f}% of the time inside the app.")

    unexpected = app_failures(cases)
    if unexpected:
        lines.append(f"- {len(unexpected)} turns failed on the app's side and were answered with the apology.")

    gaps = [case for case in cases if case["known_gap"]]
    if gaps:
        still = sum(not passed_quality(case) for case in gaps)
        lines.append(f"- {still} of {len(gaps)} runs of known-gap goldens still fail.")

    return "\n".join(lines)


def write(path=None):
    """Writes the report next to the results file and returns its summary, for the terminal."""
    path = Path(path) if path else sorted(RESULTS.glob("test_run_*.json"))[-1]
    run, cases = load(path)
    turns = all_turns(cases)
    made_with = run.get("hyperparameters") or {}

    head = (
        f"# Assistant evaluation, {path.stem.removeprefix('test_run_')}\n\n"
        f"{len(cases)} test cases with {len(turns)} turns, from `{path.name}`."
        + (" Made with " + ", ".join(f"{key}: {value}" for key, value in made_with.items()) + "." if made_with else "")
    )
    sections = [
        head,
        "## What to look at first\n\n" + summary(run, cases),
        "## Which part of a turn fails\n\nPer check: the turns it passed, of the turns it applied to. "
        "The checks are in the order a turn runs its parts.\n\n" + check_section(cases),
        "## Where a failing turn fails first\n\nA turn that fails several checks usually fails them for one reason, "
        "the first one in the order of the turn. Known gaps are left out.\n\n" + first_failure_section(cases),
        "## Which capability fails\n\nGoldens that passed every check but the latency one. Known gaps are left out.\n\n"
        + capability_section(cases),
        "## Where the time goes\n\nFrom the app's log: when each machine was asked and when it answered.\n\n" + timing_section(cases),
        "## Does the matcher's confidence point at its mistakes\n\n" + confidence_section(cases),
        "## Known gaps\n\nGoldens that show a gap `context.md` already lists. They do not fail the run.\n\n" + known_gap_section(cases),
        "## Failed goldens\n\n" + failure_section(cases),
    ]

    report = path.with_name(path.name.replace("test_run_", "report_")).with_suffix(".md")
    report.write_text("\n\n".join(sections) + "\n", encoding="utf-8", newline="\n")

    return f"\nReport: {report}\n\n{summary(run, cases)}\n"


if __name__ == "__main__":
    print(write(sys.argv[1] if len(sys.argv) > 1 else None))
