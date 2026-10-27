#!/usr/bin/env python3
"""Prepare pinned development assets for speech qualification."""
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
PINS = json.loads((ROOT / "teto-speech/assets/p3-sources.json").read_text())
DESTINATION = ROOT / "target/p3/upstream"


def main():
    for name in ["g2p", "OpenUtau"]:
        source = PINS[name]
        destination = DESTINATION / name
        destination.mkdir(parents=True, exist_ok=True)
        for record in source["files"]:
            url = f'https://raw.githubusercontent.com/{source["repository"]}/{source["revision"]}/{record["path"]}'
            target = destination / record["path"]
            if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == record["sha256"]:
                continue
            data = urllib.request.urlopen(url, timeout=60).read()
            if hashlib.sha256(data).hexdigest() != record["sha256"]:
                raise ValueError("source hash mismatch")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
    checkout = ROOT / "target/p3/OpenUtau"
    if not checkout.exists():
        subprocess.run(["git", "init", str(checkout)], check=True)
        subprocess.run(["git", "-C", str(checkout), "remote", "add", "origin",
                        "https://github.com/openutau/OpenUtau.git"], check=True)
        subprocess.run(["git", "-C", str(checkout), "fetch", "--depth", "1", "origin",
                        PINS["OpenUtau"]["revision"]], check=True)
        subprocess.run(["git", "-C", str(checkout), "checkout", "--detach", "FETCH_HEAD"], check=True)
    revision = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    if revision != PINS["OpenUtau"]["revision"]:
        raise ValueError("OpenUtau checkout revision mismatch")
    subprocess.run(["dotnet","restore",str(ROOT/"tools/openutau-oracle/Oracle.csproj"),f"-p:OpenUtauRoot={checkout}"],cwd=ROOT,check=True)
    print("Pinned speech assets are ready.")


if __name__ == "__main__":
    main()
