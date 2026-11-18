#!/usr/bin/env python3
"""Keep README.md honest.

1. Every proof link, written as [text](path#L<n> "test_name"), must point at
   an existing file that defines that name as a test (a Rust #[test] fn, a
   Python test_ method, or a function in a test_*.js script), and line <n> must
   be the line that declares it, so the link opens on the test. CI then runs
   the tests.
2. The block between <!-- quickstart --> and <!-- /quickstart --> runs
   verbatim with bash in a private copy of the committed tree (git archive
   HEAD, under the cargo target dir), and its stdout must equal the
   "Expected output" block exactly. `<number>` in the expected text matches
   one or more digits (claim ids differ on every run).

Usage: python3 -I scripts/readme_check.py [--links-only | --fix]

--fix rewrites every proof link's #L<n> anchor to the test's current line and
exits; CI never runs it.
"""

import io
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
README = os.path.join(ROOT, "README.md")

PROOF = re.compile(r'\]\(([^)\s#]+)(?:#L(\d+))? "([^"]+)"\)')
QUICKSTART = re.compile(
    r"<!-- quickstart -->\s*```bash\n(.*?)```.*?```text\n(.*?)```\s*<!-- /quickstart -->",
    re.S,
)


def defines(path, name):
    """Line number of the declaration of test `name` in `path`, or None."""
    with open(path, encoding="utf-8") as f:
        text = f.read()
    patterns = [
        # Rust: the function must carry a test attribute.
        rf"#\[(?:tokio::)?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?(fn {re.escape(name)})\b",
        # Python unittest: a test_ method.
        rf"\b(def {re.escape(name)})\b" if name.startswith("test_") else None,
        # JavaScript: a test function in a test_*.js script.
        rf"\b(function {re.escape(name)})\b" if os.path.basename(path).startswith("test_") else None,
    ]
    for p in patterns:
        match = p and re.search(p, text)
        if match:
            return text.count("\n", 0, match.start(1)) + 1
    return None


def check_links(readme):
    errors = []
    links = PROOF.findall(readme)
    for path, anchor, name in links:
        full = os.path.join(ROOT, path)
        if not os.path.isfile(full):
            errors.append(f"proof link to missing file: {path}")
            continue
        line = defines(full, name)
        if line is None:
            errors.append(f"{path} does not define test {name!r}")
        elif not anchor:
            errors.append(f"proof link to {name!r} has no #L anchor (run --fix)")
        elif int(anchor) != line:
            errors.append(f"{path}#L{anchor} is not {name!r}, which is at line {line} (run --fix)")
    if not links:
        errors.append("README has no proof links")
    return errors


def check_quickstart(readme):
    match = QUICKSTART.search(readme)
    if not match:
        return ["README has no quickstart block between the markers"]
    commands, expected = match.groups()
    # Run in a private copy of the committed tree, so the check never touches a
    # store (or anything else) in the working checkout; only that copy is removed.
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
    os.makedirs(target, exist_ok=True)
    work = tempfile.mkdtemp(prefix="readme-check-", dir=target)
    try:
        archive = subprocess.run(
            ["git", "-C", ROOT, "archive", "--format=tar", "HEAD"],
            stdout=subprocess.PIPE,
            check=True,
        ).stdout
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(work, filter="data")
        run = subprocess.run(
            ["bash", "-euo", "pipefail", "-c", commands],
            cwd=work,
            env={**os.environ, "CARGO_TARGET_DIR": target},
            stdout=subprocess.PIPE,
            text=True,
        )
    finally:
        shutil.rmtree(work, ignore_errors=True)
    if run.returncode != 0:
        return [f"quickstart exited with {run.returncode}"]
    pattern = "".join(
        r"\d+" if part == "<number>" else re.escape(part)
        for part in re.split(r"(<number>)", expected)
    )
    if not re.fullmatch(pattern, run.stdout):
        return [f"quickstart output differs.\nexpected:\n{expected}\ngot:\n{run.stdout}"]
    return []

def fix_anchors(readme):
    def anchored(match):
        path, _, name = match.groups()
        full = os.path.join(ROOT, path)
        line = defines(full, name) if os.path.isfile(full) else None
        if line is None:
            return match.group(0)  # left for check_links to report
        return f'](' + f'{path}#L{line} "{name}")'

    return PROOF.sub(anchored, readme)


def main():
    with open(README, encoding="utf-8") as f:
        readme = f.read()
    if "--fix" in sys.argv:
        with open(README, "w", encoding="utf-8") as f:
            f.write(fix_anchors(readme))
        print("README check: anchors rewritten")
        return
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
