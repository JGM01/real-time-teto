#!/usr/bin/env python3
"""Check full-line Rust comments for simplified technical English."""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = [
    "teto-llm",
    "teto-llm-protocol",
    "teto-llm-client",
    "teto-speech",
    "teto-cli",
    "teto-core",
    "earshot",
    "earshot/earshot-audit",
    "phone-spike",
    "fuzz",
]
BANNED = [
    "previously",
    "used to",
    "no longer",
    "legacy",
    "old behavior",
    "old implementation",
    "changed from",
    "we ",
    "our ",
    "todo",
    "fixme",
    "xxx",
    "hack",
    "for now",
    "temporarily",
    "will be",
]
MAX_WORDS = 25
COMMENT = re.compile(r"^\s*//+!?\s?(.*)$")


def comments(path):
    for number, line in enumerate(path.read_text().splitlines(), 1):
        match = COMMENT.match(line)
        if match:
            yield number, match.group(1).strip()


def main():
    failures = []
    for package in PACKAGES:
        for path in sorted((ROOT / package).rglob("*.rs")):
            if "target" in path.parts:
                continue
            for number, text in comments(path):
                lowered = text.lower()
                for phrase in BANNED:
                    if phrase in lowered:
                        failures.append(f"{path}:{number}: banned phrase '{phrase}'")
                for sentence in re.split(r"(?<=[.!?])\s+", text):
                    words = sentence.split()
                    if len(words) > MAX_WORDS:
                        failures.append(
                            f"{path}:{number}: sentence has {len(words)} words"
                        )
    if failures:
        print("\n".join(failures))
        return 1
    print("comments: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
