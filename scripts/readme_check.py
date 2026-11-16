#!/usr/bin/env python3
"""Keep README.md honest.

1. Every proof link, written as [text](path "test_name"), must point at an
   existing file that defines that name as a test (a Rust #[test] fn, a Python
   test_ method, or a function in a test_*.js script). CI then runs the tests.
2. The block between <!-- quickstart --> and <!-- /quickstart --> runs
   verbatim with bash from the repository root, and its stdout must equal the
   "Expected output" block exactly. `<number>` in the expected text matches
   one or more digits (claim ids differ on every run).

Usage: python3 -I scripts/readme_check.py [--links-only]
"""

import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
README = os.path.join(ROOT, "README.md")

PROOF = re.compile(r'\]\(([^)\s]+) "([^"]+)"\)')
QUICKSTART = re.compile(
    r"<!-- quickstart -->\s*```bash\n(.*?)```.*?```text\n(.*?)```\s*<!-- /quickstart -->",
    re.S,
)


def defines(path, name):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    patterns = [
        # Rust: the function must carry a test attribute.
        rf"#\[(?:tokio::)?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn {re.escape(name)}\b",
        # Python unittest: a test_ method.
        rf"\bdef {re.escape(name)}\b" if name.startswith("test_") else None,
        # JavaScript: a test function in a test_*.js script.
        rf"\bfunction {re.escape(name)}\b" if os.path.basename(path).startswith("test_") else None,
    ]
    return any(p and re.search(p, text) for p in patterns)


def check_links(readme):
    errors = []
    links = PROOF.findall(readme)
    for path, name in links:
        full = os.path.join(ROOT, path)
        if not os.path.isfile(full):
            errors.append(f"proof link to missing file: {path}")
        elif not defines(full, name):
            errors.append(f"{path} does not define test {name!r}")
    if not links:
        errors.append("README has no proof links")
    return errors


def check_quickstart(readme):
    match = QUICKSTART.search(readme)
    if not match:
        return ["README has no quickstart block between the markers"]
    commands, expected = match.groups()
    store = os.path.join(ROOT, ".aver")
    if os.path.exists(store):
        return [".aver already exists in the repository root; refusing to touch it"]
    try:
        run = subprocess.run(
            ["bash", "-euo", "pipefail", "-c", commands],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            text=True,
        )
    finally:
        shutil.rmtree(store, ignore_errors=True)
    if run.returncode != 0:
        return [f"quickstart exited with {run.returncode}"]
    pattern = "".join(
        r"\d+" if part == "<number>" else re.escape(part)
        for part in re.split(r"(<number>)", expected)
    )
    if not re.fullmatch(pattern, run.stdout):
        return [f"quickstart output differs.\nexpected:\n{expected}\ngot:\n{run.stdout}"]
    return []


def main():
    with open(README, encoding="utf-8") as f:
        readme = f.read()
    errors = check_links(readme)
    if "--links-only" not in sys.argv:
        errors += check_quickstart(readme)
    for error in errors:
        print(f"README check: {error}", file=sys.stderr)
    if errors:
        sys.exit(1)
    print("README check: ok")


if __name__ == "__main__":
    main()
