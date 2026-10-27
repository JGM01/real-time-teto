#!/usr/bin/env python3
"""Opt-in real-worker CLI tests. Temporary journals use the real Keychain adapter."""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = str(ROOT / 'target/release/teto-cli')


def environment(directory):
    env = os.environ.copy()
    env.update(TETO_MODE='text', TETO_JOURNAL='on', TETO_JOURNAL_DIR=str(directory),
               TETO_LLM_ASSETS=str(ROOT / 'models'), TETO_LOG='off')
    env.pop('TETO_WHISPER_MODEL', None)
    return env


def begin(env, prompt):
    child = subprocess.Popen([BINARY], env=env, stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    child.stdin.write(prompt.encode() + b'\n')
    child.stdin.flush()
    return child


def first_clause(child):
    assert select.select([child.stdout], [], [], 120)[0], 'first clause deadline'
    line = child.stdout.readline()
    assert line.startswith(b'clause: '), 'missing clause'
    return line


def owned_worker(parent):
    rows = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,comm='], text=True)
    matches = [int(parts[0]) for row in rows.splitlines()
               if len(parts := row.split(None, 2)) == 3 and int(parts[1]) == parent
               and parts[2].endswith('teto-llm-worker')]
    assert len(matches) == 1, 'exactly one owned worker'
    return matches[0]


def run_cases(env):
    completed = subprocess.run([BINARY], input=b'My name is Robin. Greet me briefly.\nWhat name did I give you?\n',
                               capture_output=True, env=env, timeout=150)
    assert completed.returncode == 0 and completed.stdout.count(b'clause: ') >= 2
    assert b'generation=Eos playback=Complete' in completed.stderr
    assert b'<|channel>' not in completed.stdout
    results = {'text_followup_and_eof': True}
    child = begin(env, 'Explain a forest in several complete sentences.')
    first_clause(child)
    child.send_signal(signal.SIGINT)
    _, errors = child.communicate(timeout=15)
    assert child.returncode == 0 and b'playback=Interrupted' in errors
    results['ctrl_c_during_output'] = True
    child = begin(env, 'Say a friendly greeting.')
    child.stdout.close()
    child.stdout = None
    _, errors = child.communicate(timeout=150)
    assert child.returncode == 1 and b'playback=Failed' in errors
    results['broken_stdout_finalizes_failed'] = True
    child = begin(env, 'Explain a forest in several complete sentences.')
    first_clause(child)
    os.kill(owned_worker(child.pid), signal.SIGKILL)
    _, errors = child.communicate(input=b'Continue.\n', timeout=15)
    assert child.returncode == 1 and b'Worker' in errors
    results['active_child_crash_is_service_failure'] = True
    return results


def main():
    with tempfile.TemporaryDirectory(prefix='teto-cli-real-test-') as root:
        directory = Path(root) / 'journal'
        env = environment(directory)
        subprocess.run([BINARY, 'journal-init'], env=env, check=True)
        try:
            results = run_cases(env)
            assert directory.joinpath('segment-00.cbor').stat().st_size > 0
            results['encrypted_journal_written'] = True
            print(json.dumps(results, indent=2))
        finally:
            subprocess.run([BINARY, 'journal-delete'], env=env, check=True)
        assert not directory.exists()


if __name__ == '__main__':
    main()
