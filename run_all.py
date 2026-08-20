#!/usr/bin/env python3
"""Entry script: build ms and run all .ms scripts under tests/ms/ and examples/.

Usage:
    python run_all.py [--timeout 30]

Semantics (same as the former run-all.ps1):
    - tests/ms/ collected recursively, fixtures/ helper modules skipped.
    - normal script: exit 0 -> PASS; nonzero -> FAIL (stderr shown).
    - tests/ms/negative/ scripts: nonzero exit -> PASS (expected failure),
      exit 0 -> FAIL.
    - per-script timeout kills the process and counts as FAIL; execution
      continues after failures.
    - summary + failed list at the end; exit code 1 if any failure.
"""

import argparse
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
NEGATIVE_PREFIX = "tests/ms/negative/"
FIXTURES_DIR = "fixtures"


def enable_color():
    if os.environ.get("NO_COLOR") or not sys.stdout.isatty():
        return False
    if os.name == "nt":
        import ctypes

        kernel32 = ctypes.windll.kernel32
        kernel32.SetConsoleMode(kernel32.GetStdHandle(-11), 7)
    return True


COLOR = enable_color()


def c(text, code):
    if not COLOR:
        return text
    return "\033[{}m{}\033[0m".format(code, text)


def collect_scripts():
    scripts = []
    base = os.path.join(ROOT, "tests", "ms")
    for dirpath, dirnames, filenames in os.walk(base):
        if os.path.basename(dirpath) == FIXTURES_DIR:
            dirnames[:] = []
            continue
        for name in filenames:
            if name.endswith(".ms"):
                scripts.append(os.path.join(dirpath, name))
    examples = os.path.join(ROOT, "examples")
    for name in os.listdir(examples):
        if name.endswith(".ms"):
            scripts.append(os.path.join(examples, name))
    scripts.sort()
    return scripts


def main():
    parser = argparse.ArgumentParser(description="Run all mslang test scripts")
    parser.add_argument("--timeout", type=int, default=30, help="per-script timeout in seconds")
    args = parser.parse_args()

    if not shutil.which("cargo"):
        print(c("cargo not found in PATH", "31"))
        return 1

    print("==> cargo build --bin ms")
    build = subprocess.run(["cargo", "build", "--bin", "ms"], cwd=ROOT)
    if build.returncode != 0:
        print(c("build FAILED", "31"))
        return 1

    exe = "ms.exe" if os.name == "nt" else "ms"
    ms = os.path.join(ROOT, "target", "debug", exe)

    scripts = collect_scripts()
    if not scripts:
        print(c("no .ms scripts found", "31"))
        return 1

    passed = failed = 0
    failed_cases = []

    for i, script in enumerate(scripts, 1):
        rel = os.path.relpath(script, ROOT).replace(os.sep, "/")
        negative = rel.startswith(NEGATIVE_PREFIX)
        print()
        print(c("=== [{}/{}] {}".format(i, len(scripts), rel), "36"))

        timed_out = False
        try:
            proc = subprocess.run(
                [ms, "run", script],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=args.timeout,
            )
            code, out, err = proc.returncode, proc.stdout, proc.stderr
        except subprocess.TimeoutExpired:
            timed_out = True
            code, out, err = None, b"", b""

        out = out.decode("utf-8", errors="replace").rstrip()
        err = err.decode("utf-8", errors="replace").rstrip()
        if out:
            print(out)

        if timed_out:
            print(c("FAIL (timeout)", "31"))
            if err:
                print(c(err, "31"))
            failed += 1
            failed_cases.append("{} (timeout)".format(rel))
        elif negative:
            if code != 0:
                print(c("PASS (expected failure, exit {})".format(code), "32"))
                if err:
                    print(c(err, "90"))
                passed += 1
            else:
                print(c("FAIL (negative case exited 0, expected nonzero)", "31"))
                failed += 1
                failed_cases.append("{} (expected failure but exited 0)".format(rel))
        else:
            if code == 0:
                print(c("PASS", "32"))
                passed += 1
            else:
                print(c("FAIL (exit {})".format(code), "31"))
                if err:
                    print(c(err, "31"))
                failed += 1
                failed_cases.append("{} (exit {})".format(rel, code))

    print()
    print("=== summary: {} passed, {} failed / {} total".format(passed, failed, len(scripts)))
    if failed_cases:
        print("failed cases:")
        for case in failed_cases:
            print(c("  " + case, "31"))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
