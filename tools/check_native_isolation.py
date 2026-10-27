#!/usr/bin/env python3
"""Check both resolved trees and real Mach-O symbols after make build-release."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def symbols(name):
    return subprocess.check_output(['nm', '-gU', str(ROOT / 'target/release' / name)], text=True)

parent = symbols('teto-cli')
child = symbols('teto-llm-worker')
checks = {
    'parent_contains_whisper': '_whisper_init_from_file_with_params' in parent,
    'child_contains_llama': '_llama_model_load_from_file' in child,
    'parent_excludes_llama': '_llama_' not in parent and 'llama_cpp' not in parent,
    'child_excludes_whisper': '_whisper_' not in child and 'whisper_rs' not in child,
    'child_excludes_earshot': 'earshot' not in child,
}
print(json.dumps(checks, indent=2))
raise SystemExit(0 if all(checks.values()) else 1)
