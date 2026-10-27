#!/usr/bin/env python3
"""Check that every invariant row names a test that exists."""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = [(ROOT / "llm-subsystem.md", "LLM"), (ROOT / "docs/speech.md", "SPCH")]
ROW = re.compile(r"^\| ((?:LLM|SPCH)-\d{3}) \| .* \| `([^`]+)` \| `([^`]+)` \| Required \|$")


def main():
    rows = []
    for document, prefix in DOCUMENTS:
        current = []
        for line in document.read_text().splitlines():
            match = ROW.match(line)
            if match:
                current.append(match.groups())
        identifiers = [int(row[0].split("-")[1]) for row in current]
        if not current or identifiers != list(range(1, len(current) + 1)) or any(not row[0].startswith(prefix + "-") for row in current):
            print(f"invariants: invalid identifiers in {document.name}")
            return 1
        rows.extend(current)
    result = subprocess.run(
        ["cargo", "test", "--workspace", "--", "--list"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    speech = subprocess.run(
        ["cargo", "test", "-p", "teto-speech", "--features", "cmudict,g2p-gru,voicebank-wav,llm-adapter", "--", "--list"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    output = result.stdout + result.stderr + speech.stdout + speech.stderr
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
