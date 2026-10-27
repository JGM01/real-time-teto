#!/usr/bin/env python3
"""Summarize measured metadata. The tool never opens or decrypts a journal."""
import json
import math
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "teto-llm/qualification"


def records(path):
    result = []
    for line in path.read_text().splitlines():
        if line.startswith("{"):
            try:
                result.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return result


def percentile(values, percent):
    ordered = sorted(value for value in values if value is not None)
    if not ordered:
        return None
    return ordered[math.ceil(len(ordered) * percent / 100) - 1]


def microseconds(row, name):
    value = row.get(name + "_us")
    if value is not None:
        return value
    value = row.get(name + "_ms")
    return None if value is None else value * 1000


def throughput(turns):
    microseconds_total = 0.0
    tokens = 0
    for turn in turns:
        metrics = turn["metrics"]
        microseconds_total += metrics.get("decode_us", metrics.get("generation_ms", 0) * 1000)
        tokens += metrics["generated_tokens"]
    return tokens * 1_000_000 / microseconds_total if microseconds_total else None


def desktop(prefix):
    log = DATA / (prefix + "-run.log")
    resources = DATA / (prefix + "-resources.json")
    if not log.exists():
        return {"status": "pending"}
    rows = records(log)
    turns = [row for row in rows if "qualification_turn" in row]
    resource = json.loads(resources.read_text()) if resources.exists() else {}
    latencies = [microseconds(row, "first_clause_at_parent") for row in turns]
    warm = latencies[1:101]
    cache_hits = [
        microseconds(row, "first_clause_at_parent")
        for row in turns
        if row["metrics"]["cached_prefix_tokens"] > 0
    ]
    return {
        "exit_code": resource.get("exit_code"),
        "turns": len(turns),
        "session_summary": next((row for row in rows if "qualification_summary" in row), None),
        "warm_p50_us": percentile(warm, 50),
        "warm_p95_us": percentile(warm, 95),
        "all_turns_p50_us": percentile(latencies, 50),
        "all_turns_p95_us": percentile(latencies, 95),
        "cache_hit_p95_us": percentile(cache_hits, 95),
        "cache_hit_turns": len(cache_hits),
        "generation_tokens_per_second": throughput(turns),
        "generation_failures": sum(row["generation_stop"] == "Failed" for row in turns),
        "interrupted_playback": sum(row["playback_stop"] == "Interrupted" for row in turns),
        "actions": [row for row in rows if "qualification_action" in row],
        "evicted_exchanges": sum(
            row["metrics"].get("evicted_exchanges", row["metrics"].get("evicted_turns", 0))
            for row in turns
        ),
        "context_rebuilds": sum(row["metrics"].get("context_rebuilds", 0) for row in turns),
        "peak_parent_child_rss_bytes": resource.get("peak_parent_child_rss_bytes"),
        "thermal_state_values": sorted(
            {row["thermal_state_raw"] for row in resource.get("samples", [])}
        ),
        "binary_sha256": resource.get("binary_sha256", {}),
    }


def phone_runs():
    result = []
    for path in sorted(
        (DATA / "phone-pro-max").glob("spike-*.jsonl"),
        key=lambda item: item.stat().st_mtime,
    ):
        if path.name.endswith("-resources.jsonl"):
            continue
        rows = records(path)
        turns = [row for row in rows if "qualification_turn" in row or "turn" in row]
        resources = records(path.with_name(path.stem + "-resources.jsonl"))
        sampled = len(resources) >= 5
        result.append(
            {
                "report": path.name,
                "measured_turns": len(turns),
                "resource_samples": len(resources),
                "cache_modes": sorted({row["metrics"]["cache_mode"] for row in turns}),
                "context_rebuilds": sum(row["metrics"].get("context_rebuilds", 0) for row in turns),
                "evicted_exchanges": sum(
                    row["metrics"].get("evicted_exchanges", 0) for row in turns
                ),
                "generation_failures": sum(
                    row["generation_stop"] == "Failed" for row in turns
                ),
                "summary": next(
                    (
                        row
                        for row in rows
                        if "qualification_summary" in row or "measurement_summary" in row
                    ),
                    None,
                ),
                "error": next((row for row in rows if "spike_error" in row), None),
                "load_us": microseconds(turns[0], "load") if turns else None,
                "first_clause_p50_us": percentile(
                    [microseconds(row["metrics"], "first_clause") for row in turns], 50
                ),
                "first_clause_p95_us": percentile(
                    [microseconds(row["metrics"], "first_clause") for row in turns], 95
                ),
                "generation_tokens_per_second": throughput(turns),
                "peak_resident_bytes": (
                    max(
                        (row["resident_bytes"] for row in resources if "resident_bytes" in row),
                        default=None,
                    )
                    if sampled
                    else None
                ),
                "peak_footprint_bytes": (
                    max(
                        (
                            row["physical_footprint_bytes"]
                            for row in resources
                            if "physical_footprint_bytes" in row
                        ),
                        default=None,
                    )
                    if sampled
                    else None
                ),
                "thermal_state_values": (
                    sorted({row["thermal_state"] for row in resources}) if sampled else []
                ),
                "probe": next((row for row in rows if "cancellation_probe" in row), None),
            }
        )
    return result


def phone_cache_probe():
    paths = sorted(
        (DATA / "phone-pro-max").glob("cache-*.jsonl"),
        key=lambda item: item.stat().st_mtime,
    )
    if not paths:
        return {"status": "pending"}
    path = paths[-1]
    report = json.loads(path.read_text())
    return {
        "report": path.name,
        "qualified": report.get("qualified"),
        "device": report.get("device"),
        "os": report.get("os"),
        "model_sha256": report.get("model_sha256"),
        "template_sha256": report.get("template_sha256"),
        "cases": report.get("cases", []),
    }


def main():
    policy = (ROOT / "teto-llm-protocol/src/policy.rs").read_text()
    names = [
        "MODEL_SHA256",
        "PHONE_SHA256",
        "DESKTOP_TEMPLATE_SHA256",
        "PHONE_TEMPLATE_SHA256",
        "RUNTIME",
    ]
    identity = {
        name: re.search(
            r"pub const " + name + r': &str\s*=\s*"([^"]+)"', policy
        ).group(1)
        for name in names
    }
    cold_path = DATA / "desktop-cold-policy2.log"
    if not cold_path.exists():
        cold_path = DATA / "desktop-cold-run.log"
    cold = records(cold_path) if cold_path.exists() else []
    result = {
        "identity": identity,
        "percentile_method": "nearest rank",
        "audio": "pending: real TTS required",
        "cold_process": next((row for row in cold if "cold_process_summary" in row), None),
        "desktop": desktop("desktop-policy2"),
        "lifecycle": {"status": "pending"},
        "phone_pro_max": phone_runs(),
        "phone_cache_probe": phone_cache_probe(),
    }
    (DATA / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
