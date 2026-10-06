"""
    Runs the suites with `deepeval test run` and writes the report of the run.

        python run.py                       every suite
        python run.py test_form.py          one suite
        python run.py -r 3                  every golden three times, for pass rates instead of one sample
        python run.py test_form.py -k de    whatever else pytest takes

    Run it with the Python of the suites' virtual environment (.venv), see the README.
"""
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "results"

ENVIRONMENT = {
    # Kept on this machine: the results hold what callers said
    "DEEPEVAL_TELEMETRY_OPT_OUT": "1",
    "DEEPEVAL_RESULTS_FOLDER": str(RESULTS),
    # Not the project's .env: that one is the app's, and the suites read EVAL_* from the shell
    "DEEPEVAL_DISABLE_DOTENV": "1",
    "PYTHONUTF8": "1",
}


def main():
    arguments = sys.argv[1:]
    # `deepeval test run` takes what to run first: this folder, unless a test file is named
    is_target = lambda argument: argument.endswith(".py") or "::" in argument
    target = next((argument for argument in arguments if is_target(argument)), ".")
    options = [argument for argument in arguments if argument != target]
    # DeepEval prints a table of every test case at the end. The report says the same in less
    if not {"-d", "--display"} & set(options):
        options += ["--display", "failing"]

    os.environ.update({key: os.environ.get(key, value) for key, value in ENVIRONMENT.items()})
    import report

    deepeval = Path(sys.executable).with_name("deepeval.exe" if os.name == "nt" else "deepeval")
    before = set(RESULTS.glob("test_run_*.json"))
    exit_code = subprocess.run([str(deepeval), "test", "run", target, *options], cwd=HERE).returncode

    written = sorted(set(RESULTS.glob("test_run_*.json")) - before)
    if written:
        print(report.write(written[-1]))
    else:
        print("The run saved no results, so there is no report.")

    sys.exit(exit_code)


if __name__ == "__main__":
    main()
