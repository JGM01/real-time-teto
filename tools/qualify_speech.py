#!/usr/bin/env python3
"""Qualify the speech frontend and unit planner against local assets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "target/p3"
PINS = json.loads((ROOT / "teto-speech/assets/p3-sources.json").read_text())
FEATURES = "cmudict,g2p-gru,voicebank-wav,llm-adapter"


def run(command, **kwargs):
    if command[0]=="cargo":
        command=["rustup","run","1.88.0",*command]
        environment=kwargs.pop("env",dict(os.environ))
        environment["RUSTC"]=subprocess.check_output(["rustup","which","--toolchain","1.88.0","rustc"],text=True).strip()
        environment["RUSTDOC"]=subprocess.check_output(["rustup","which","--toolchain","1.88.0","rustdoc"],text=True).strip()
        kwargs["env"]=environment
    return subprocess.run(command, cwd=ROOT, check=True, **kwargs)


def check_sources(checkout):
    for name, expected in PINS["corpora"].items():
        data = (ROOT / "teto-speech/tests/fixtures" / name).read_bytes()
        if hashlib.sha256(data).hexdigest() != expected:
            raise ValueError(f"frozen corpus hash mismatch: {name}")
    revision = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    if revision != PINS["OpenUtau"]["revision"]:
        raise ValueError("OpenUtau checkout revision mismatch")
    for record in PINS["OpenUtau"]["files"]:
        path = checkout / record["path"]
        if not path.exists() and record["path"] == "LICENSE.txt":
            path = checkout / "LICENSE"
        if hashlib.sha256(path.read_bytes().replace(b"\r\n",b"\n")).hexdigest() != record["sha256"]:
            raise ValueError("OpenUtau source hash mismatch")


def rust_report(bank, checkpoint, corpus, name):
    path = OUTPUT / name
    with path.open("w") as stream:
        run(["cargo", "run", "--offline", "--release", "-q", "-p", "teto-speech", "--example", "p3",
             "--features", FEATURES, "--", str(bank), str(corpus), str(checkpoint)], stdout=stream)
    return json.loads(path.read_text())


def check_parity(actual, reference):
    if len(reference) != 50 or len(actual["records"]) != 50:
        raise ValueError("parity corpus size mismatch")
    repaired = 0
    for a, b in zip(actual["records"], reference):
        if a["case"] != b["case"] or a["reference_groups"] != b["groups"]:
            raise ValueError(f'OpenUtau alias group mismatch at case {a["case"]}')
        allowed = PINS["allowed_coverage_repairs"].get(str(a["case"]), [])
        if a["repairs"] != allowed:
            raise ValueError("coverage repair differs from the explicit allowance")
        covered = set()
        for (start, end), repair in zip(a["unit_phones"], a["unit_repairs"]):
            if not repair:
                covered.update(range(start, end))
        missing = set(range(len(a["phones"]))) - covered
        expected = {p for start, end in allowed for p in range(start, end)}
        if missing != expected:
            raise ValueError("coverage repair does not preserve an otherwise unrepresented phone")
        repaired += bool(allowed)
    return {"cases": 50, "exact_cases": 50 - repaired, "cases_with_coverage_repairs": repaired}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bank", type=Path, required=True)
    parser.add_argument("--checkpoint", type=Path, default=OUTPUT / "upstream/g2p/g2p_en/checkpoint20.npz")
    parser.add_argument("--oracle-python", type=Path, default=OUTPUT / "oracle-venv/bin/python")
    parser.add_argument("--openutau", type=Path, default=OUTPUT / "OpenUtau")
    args = parser.parse_args()
    args.bank = args.bank.resolve()
    with args.bank.open("rb") as bank_file:
        if hashlib.file_digest(bank_file,"sha256").hexdigest()!="addb3ab9dbe3dce7cb40fe6ba0c93ab5814d02acf5b091d0a762de370b813d6e":
            raise ValueError("official bank checksum mismatch")
    check_sources(args.openutau)
    oracle = OUTPUT / "g2p-oracle.json"
    run([str(args.oracle_python), "tools/g2p_oracle.py", str(args.checkpoint),
         "teto-speech/tests/fixtures/g2p-words.txt", str(oracle)])
    environment = dict(os.environ, TETO_G2P_CHECKPOINT=str(args.checkpoint.resolve()),
                       TETO_G2P_ORACLE=str(oracle), TETO_VOICEBANK_DIR=str(args.bank))
    run(["cargo", "test", "--offline", "--release", "-p", "teto-speech", "--features", FEATURES,
         "--test", "gru", "--test", "voicebank", "--", "--ignored"], env=environment)
    coverage = rust_report(args.bank, args.checkpoint, ROOT / "teto-speech/tests/fixtures/coverage-1050.txt", "coverage.json")
    actual = rust_report(args.bank, args.checkpoint, ROOT / "teto-speech/tests/fixtures/parity-50.txt", "parity-rust.json")
    if coverage["cases"] != 1050 or coverage["failed"]:
        raise ValueError("incomplete supported corpus coverage")
    with zipfile.ZipFile(args.bank) as bank:
        oto = bank.read(next(n for n in bank.namelist() if n.endswith("/oto.ini"))).decode("utf-8-sig")
    actual["inventory"] = sorted({line.split("=", 1)[1].split(",")[0] for line in oto.splitlines() if "=" in line})
    input_path = OUTPUT / "oracle-input.json"
    input_path.write_text(json.dumps(actual))
    reference_path = OUTPUT / "parity-openutau.json"
    with (OUTPUT / "dotnet-build.log").open("w") as log:
        run(["dotnet", "build", "tools/openutau-oracle/Oracle.csproj", "--no-restore", "--disable-build-servers", "-p:UseSharedCompilation=false", "-m:1", "-c", "Release",
             f"-p:OpenUtauRoot={args.openutau.resolve()}"], stdout=log, stderr=subprocess.STDOUT)
    run(["dotnet", str(OUTPUT / "dotnet/bin/Release/net10.0/Oracle.dll"), str(input_path), str(reference_path)])
    parity = check_parity(actual, json.loads(reference_path.read_text()))
    summary = {"coverage": {k: v for k, v in coverage.items() if k != "records"}, "parity": parity,
               "g2p_cases": len(json.loads(oracle.read_text())["records"])}
    (OUTPUT / "qualification.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
