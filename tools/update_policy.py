"""Read `make --silent policy` on stdin and update only the generated table."""
from pathlib import Path
import sys

path = Path(__file__).resolve().parents[1] / "llm-subsystem.md"
table = sys.stdin.read()
if not table.startswith("| Decision | Policy V1 |\n"):
    raise SystemExit("expected policy table on stdin")
before, rest = path.read_text().split("<!-- POLICY:BEGIN -->\n", 1)
_, after = rest.split("<!-- POLICY:END -->", 1)
path.write_text(before + "<!-- POLICY:BEGIN -->\n" + table + "<!-- POLICY:END -->" + after)
