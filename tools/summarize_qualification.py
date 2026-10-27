#!/usr/bin/env python3
"""Summarize measured metadata only. Never open/decrypt conversation journals."""
import json
import math
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / 'teto-llm/qualification'


def records(path):
    result = []
    for line in path.read_text().splitlines():
        if line.startswith('{'):
            try:
                result.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return result


def percentile(values, percent):
    ordered = sorted(value for value in values if value is not None)
    return ordered[math.ceil(len(ordered) * percent / 100) - 1] if ordered else None


def throughput(turns):
    milliseconds = sum(turn['metrics']['generation_ms'] for turn in turns)
    return sum(turn['metrics']['generated_tokens'] for turn in turns) * 1000 / milliseconds if milliseconds else None


def desktop(prefix):
    log = DATA / (prefix + '-run.log')
    resources = DATA / (prefix + '-resources.json')
    if not resources.exists():
        return {'status': 'pending'}
    rows = records(log)
    turns = [row for row in rows if 'qualification_turn' in row]
    resource = json.loads(resources.read_text())
    latencies = [row['first_clause_at_parent_ms'] for row in turns]
    warm = latencies[1:101]
    cache_hits = [row for row in turns if row['metrics']['cached_prefix_tokens'] > 0]
    return {'exit_code': resource['exit_code'], 'turns': len(turns),
            'session_summary': next((row for row in rows if 'qualification_summary' in row), None),
            'first100_warm_p50_ms': percentile(warm, 50), 'first100_warm_p95_ms': percentile(warm, 95),
            'all_turns_p50_ms': percentile(latencies, 50), 'all_turns_p95_ms': percentile(latencies, 95),
            'cache_hit_p95_ms': percentile([row['first_clause_at_parent_ms'] for row in cache_hits], 95),
            'cache_hit_turns': len(cache_hits), 'generation_tokens_per_second': throughput(turns),
            'generation_failures': sum(row['generation_stop'] == 'Failed' for row in turns),
            'interrupted_playback': sum(row['playback_stop'] == 'Interrupted' for row in turns),
            'actions': [row for row in rows if 'qualification_action' in row],
            'evicted_exchanges': sum(row['metrics']['evicted_turns'] for row in turns),
            'peak_parent_child_rss_bytes': resource['peak_parent_child_rss_bytes'],
            'thermal_state_values': sorted({row['thermal_state_raw'] for row in resource['samples']}),
            'binary_sha256': resource['binary_sha256']}


def phone_runs():
    result = []
    for path in sorted((DATA / 'phone-pro-max').glob('spike-*.jsonl')):
        if path.name.endswith('-resources.jsonl'):
            continue
        rows = records(path)
        turns = [row for row in rows if 'turn' in row]
        resources = records(path.with_name(path.stem + '-resources.jsonl'))
        result.append({'report': path.name, 'measured_turns': len(turns),
                       'summary': next((row for row in rows if 'measurement_summary' in row), None),
                       'error': next((row for row in rows if 'spike_error' in row), None),
                       'load_ms': turns[0]['load_ms'] if turns else None,
                       'first_clause_p50_ms': percentile([row['metrics']['first_clause_ms'] for row in turns], 50),
                       'first_clause_p95_ms': percentile([row['metrics']['first_clause_ms'] for row in turns], 95),
                       'generation_tokens_per_second': throughput(turns),
                       'peak_resident_bytes': max((row['resident_bytes'] for row in resources if 'resident_bytes' in row), default=None),
                       'peak_footprint_bytes': max((row['physical_footprint_bytes'] for row in resources if 'physical_footprint_bytes' in row), default=None),
                       'thermal_state_values': sorted({row['thermal_state'] for row in resources}),
                       'probe': next((row for row in rows if 'cancellation_probe' in row), None)})
    return result


def main():
    policy = (ROOT / 'teto-llm-protocol/src/policy.rs').read_text()
    names = ['MODEL_SHA256', 'PHONE_SHA256', 'DESKTOP_TEMPLATE_SHA256', 'PHONE_TEMPLATE_SHA256', 'RUNTIME']
    identity = {name: re.search(r'pub const ' + name + r': &str\s*=\s*"([^"]+)"', policy).group(1) for name in names}
    cold = records(DATA / 'desktop-cold-run.log')
    result = {'identity': identity, 'percentile_method': 'nearest rank', 'audio': 'pending: real TTS required',
              'cold_process': next(row for row in cold if 'cold_process_summary' in row),
              'desktop': desktop('desktop-final'), 'lifecycle': desktop('desktop-lifecycle'), 'phone_pro_max': phone_runs()}
    (DATA / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
