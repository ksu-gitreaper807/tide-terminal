#!/usr/bin/env python3
"""Run the local automated library and preserve honest, independently labelled results."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True, help='Explicit executable to test with the CLI suites')
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--target-dir', type=Path, help='Reuse a Cargo build directory')
    parser.add_argument('--property-cases', type=int, default=256)
    parser.add_argument('--seed', type=int, default=20261112)
    parser.add_argument('--gui', action='store_true', help='Also run GUI checks on an already isolated accelerated display')
    parser.add_argument('--kwin-input-helper', type=Path)
    parser.add_argument('--btop-binary', type=Path)
    parser.add_argument('--eza-binary', type=Path)
    args = parser.parse_args()
    if args.property_cases < 1 or not 0 <= args.seed < 2**64:
        parser.error('Property cases must be positive and seed must fit a u64')
    app = Path(__file__).resolve().parent.parent
    binary = args.binary.resolve(strict=True)
    output = args.output_dir.resolve()
    scratch = args.work_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    scratch.mkdir(parents=True, exist_ok=True)
    if args.gui and not os.environ.get('DISPLAY'):
        parser.error('--gui requires an isolated accelerated DISPLAY; see tests/README.md')
    env = os.environ.copy()
    # Do not accidentally read or write the caller's data or send desktop notifications.
    for key in ('TIDE_HISTORY', 'TIDE_PRIVATE', 'TIDE_SHELL', 'TIDE_EXE', 'TIDE_APPEND_SOCKET', 'TIDE_APPEND_TEST_BINARY', 'PYTHONOPTIMIZE'):
        env.pop(key, None)
    env.update(PROPTEST_CASES=str(args.property_cases), PROPTEST_RNG_SEED=str(args.seed),
               TIDE_APPEND_TEST_BINARY=str(binary))
    env['CARGO_TARGET_DIR'] = str(args.target_dir.resolve() if args.target_dir else scratch / 'target')
    terminal = json.loads((app / 'tests/cases/terminal.json').read_text())
    observer = json.loads((app / 'tests/cases/observer.json').read_text())
    known = json.loads((app / 'tests/cases/known-gaps.json').read_text())
    known.update(json.loads((app / 'tests/cases/stream-known-gaps.json').read_text()))
    report = {
        'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'standalone_launcher_binary': str(app / 'bin/tide-terminal'),
        'source_manifest': str(app / 'Cargo.toml'),
        'property_cases_per_property': args.property_cases, 'property_seed': args.seed,
        'corpus': {'terminal_cases': len(terminal), 'terminal_replay_strides': [1, 3, 11, 'whole'],
                   'observer_cases': len(observer),
                   'observer_split_boundary_replays': sum(len(c['stream'].encode()) + 1 for c in observer)},
        'known_gaps': known, 'steps': {},
        'not_run': ['external vttest visual review', 'external esctest2', 'coverage-guided fuzzing',
                    'LLVM line/branch coverage', 'mutation testing'],
    }
    launcher_binary = app / 'bin/tide-terminal'
    if launcher_binary.exists():
        report['standalone_launcher_binary_sha256'] = hashlib.sha256(launcher_binary.read_bytes()).hexdigest()
    def save():
        (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    save()
    def run(name, command, timeout):
        print(f'Running {name}', flush=True)
        started = time.monotonic()
        log = output / f'{name}.log'
        with log.open('wb') as stream:
            try:
                done = subprocess.run(command, cwd=app, env=env, stdout=stream,
                                      stderr=subprocess.STDOUT, timeout=timeout)
                code = done.returncode
                status = 'passed' if code == 0 else 'failed'
            except subprocess.TimeoutExpired:
                code = None
                status = 'timeout'
            except OSError as error:
                stream.write(str(error).encode())
                code = None
                status = 'failed'
        report['steps'][name] = {'status': status, 'returncode': code,
                                 'duration_seconds': round(time.monotonic() - started, 3),
                                 'command': [str(item) for item in command], 'log': str(log)}
        if name == 'rust-tests':
            summaries = re.findall(r'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored', log.read_text())
            report['rust_test_totals'] = dict(zip(('passed', 'failed', 'ignored'),
                                                (sum(int(row[i]) for row in summaries) for i in range(3))))
        save()
        print(f'{name}: {status}', flush=True)
        return status == 'passed'
    checks = [
        ('format', ['cargo', 'fmt', '--check'], 60),
        ('clippy', ['cargo', 'clippy', '--all-targets', '--locked', '--', '-D', 'warnings'], 300),
        ('rust-tests', ['cargo', 'test', '--all-targets', '--locked', '--', '--test-threads=1'], 300),
        ('cli', [sys.executable, app / 'scripts/cli_smoke.py', '--binary', binary,
                 '--work-dir', scratch, '--report', output / 'cli.json'], 60),
        ('timer', [sys.executable, app / 'scripts/timer_smoke.py', '--binary', binary,
                   '--launcher', app / 'T_:', '--work-dir', scratch, '--report', output / 'timer.json'], 60),
    ]
    if args.gui:
        command = [sys.executable, app / 'scripts/gui_smoke.py', '--binary', binary,
                   '--work-dir', scratch, '--output-dir', output / 'gui']
        for option in ('kwin_input_helper', 'btop_binary', 'eza_binary'):
            value = getattr(args, option)
            if value:
                command.extend(['--' + option.replace('_', '-'), value.resolve()])
        checks.append(('gui', command, 300))
    else:
        report['not_run'].append('GUI / physical display and iGPU rendering')
    success = True
    for name, command, timeout in checks:
        success = run(name, command, timeout) and success
    report['completed_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    report['status'] = 'passed_with_known_gaps' if success else 'failed'
    save()
    print(f'Report: {output / "summary.json"}', flush=True)
    return 0 if success else 1


if __name__ == '__main__':
    raise SystemExit(main())
