#!/usr/bin/env python3
"""Run the real desktop soak and sample process RSS/thermal status; never capture text."""
import hashlib
import json
import sys
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'teto-llm' / 'qualification'


def processes(parent):
    rows = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss='], text=True)
    parsed = [tuple(map(int, row.split())) for row in rows.splitlines()]
    owned = {parent}
    for _ in range(16):
        newer = owned | {pid for pid, ppid, _ in parsed if ppid in owned}
        if newer == owned:
            break
        owned = newer
    return sum(rss * 1024 for pid, _, rss in parsed if pid in owned)


def main():
    OUTPUT.mkdir(exist_ok=True)
    kind = sys.argv[1] if len(sys.argv) == 2 else 'warm'
    if kind not in {'warm', 'cold', 'lifecycle'}: raise SystemExit('choose warm, cold or lifecycle')
    prefix = {'warm':'desktop-final', 'cold':'desktop-cold', 'lifecycle':'desktop-lifecycle'}[kind]
    if (OUTPUT / (prefix + '-resources.json')).exists(): raise SystemExit('preserve existing measurement; choose a new output directory')
    if kind in {'warm', 'lifecycle'}:
        with Path('/tmp/teto-final-prewarm-check.log').open('w') as log:
            subprocess.run(['make', 'check', 'build-release'], cwd=ROOT, check=True, stdout=log, stderr=subprocess.STDOUT)
    env = os.environ.copy()
    env.update(TETO_LLM_ASSETS=str(ROOT / 'models'),
               TETO_WHISPER_MODEL=str(ROOT / 'models' / 'ggml-small.en.bin'),
               TETO_JOURNAL_DIR=str(Path(tempfile.mkdtemp(prefix='teto-qualification-')) / 'journal'),
               TETO_JOURNAL='on', TETO_LOG='off',
               TETO_ASSET_CACHE=str(Path(tempfile.gettempdir()) / 'teto-asset-cache'))
    binary = str(ROOT / 'target/release/teto-cli')
    subprocess.run([binary, 'journal-init'], env=env, check=True)
    started = time.monotonic()
    peak = 0
    samples = []
    with (OUTPUT / (prefix + '-run.log')).open('w') as log:
        child = subprocess.Popen([binary, {'warm':'qualify', 'cold':'qualify-cold', 'lifecycle':'qualify-lifecycle'}[kind]], env=env, stdout=subprocess.DEVNULL, stderr=log)
        while child.poll() is None:
            rss = processes(child.pid)
            peak = max(peak, rss)
            thermal = subprocess.run(['pmset', '-g', 'therm'], capture_output=True, text=True).stdout.strip()
            state = subprocess.check_output(['/tmp/teto-thermal'], text=True).strip()
            samples.append({'thermal_state_raw': int(state), 'seconds': round(time.monotonic() - started, 1),
                            'parent_child_rss_bytes': rss, 'thermal': thermal})
            time.sleep(5)
    digests = {}
    for name in ['teto-cli', 'teto-llm-worker']:
        digest = hashlib.sha256()
        with (ROOT / 'target/release' / name).open('rb') as file:
            while chunk := file.read(1024 * 1024): digest.update(chunk)
        digests[name] = digest.hexdigest()
    result = {'binary_sha256': digests, 'kind': kind, 'exit_code': child.returncode, 'elapsed_seconds': time.monotonic() - started,
              'peak_parent_child_rss_bytes': peak, 'samples': samples,
              'journal_directory': env['TETO_JOURNAL_DIR'], 'journal': 'enabled',
              'audio': 'pending: real TTS required', 'device': subprocess.check_output(['/usr/sbin/sysctl','-n','hw.model'],text=True).strip(),
              'os': subprocess.check_output(['/usr/bin/sw_vers','-productVersion'],text=True).strip(),
              'memory_measurement': 'summed process RSS sampled every five seconds; shared pages may be counted twice'}
    (OUTPUT / (prefix + '-resources.json')).write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key != 'samples'}))
    raise SystemExit(child.returncode)


if __name__ == '__main__':
    main()
