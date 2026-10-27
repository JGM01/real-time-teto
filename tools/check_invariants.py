#!/usr/bin/env python3
"""Check that every invariant row names a test that exists."""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCUMENT = ROOT / "llm-subsystem.md"
ROW = re.compile(r"^\| (LLM-\d{3}) \| .* \| `([^`]+)` \| `([^`]+)` \| Required \|$")


def main():
    rows = []
    for line in DOCUMENT.read_text().splitlines():
        match = ROW.match(line)
        if match:
            rows.append(match.groups())
    if not rows:
        print("invariants: no rows found")
        return 1
    identifiers = [int(row[0].split("-")[1]) for row in rows]
    if identifiers != list(range(1, len(rows) + 1)):
        print("invariants: identifiers are not contiguous")
        return 1
    result = subprocess.run(
        ["cargo", "test", "--workspace", "--", "--list"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    output = result.stdout + result.stderr
    tests = {
        line.removesuffix(": test").strip().rsplit("::", 1)[-1]
        for line in output.splitlines()
        if line.endswith(": test")
    }
    missing = [row[2] for row in rows if row[2] not in tests]
    if missing:
        print("invariants: missing tests: " + ", ".join(sorted(set(missing))))
        return 1
    print(f"invariants: {len(rows)} rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
