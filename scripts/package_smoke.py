#!/usr/bin/env python3
"""Exercise an installed/relocated payload, never the user's profile or history.

GUI checks need a dedicated display, just like gui_smoke.py. No input is injected.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prefix', type=Path, required=True, help='Installed usr-style prefix')
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--gui', action='store_true')
    parser.add_argument('--gpu', choices=['integrated', 'software'], default='integrated')
    args = parser.parse_args()
    prefix = args.prefix.resolve(strict=True)
    binary = prefix / 'bin/tide-terminal'
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix='tide-package-', dir=args.work_dir) as temp:
        scratch = Path(temp)
        env = os.environ.copy()
        for name in ['TIDE_EXE', 'TIDE_ASSETS_DIR', 'TIDE_APPEND_SOCKET', 'TIDE_PRIVATE', 'TIDE_SHELL', 'TIDE_HISTORY']:
            env.pop(name, None)
        profile = scratch / 'profile.toml'
        profile.write_text(f'gpu = "{args.gpu}"\nnotifications = false\nprivate = true\nshell = "/bin/bash"\n')
        env.update(TIDE_CONFIG=str(profile), XDG_DATA_HOME=str(scratch / 'data'), XDG_CACHE_HOME=str(scratch / 'cache'))
        required = ['bin/tide-terminal', 'bin/T_:', 'bin/A_:', 'share/applications/tide-terminal.desktop',
                    'share/icons/hicolor/scalable/apps/tide-terminal.svg', 'share/licenses/tide-terminal/LICENSE',
                    'share/licenses/tide-terminal/NerdFonts-LICENSE.txt', 'share/licenses/tide-terminal/DejaVu-LICENSE.txt',
                    'share/licenses/tide-terminal/Noto-LICENSE.txt', 'share/licenses/tide-terminal/LICENSE-MIT',
                    'share/tide-terminal/vendor/blesh/ble.sh', 'share/tide-terminal/vendor/blesh/licenses/LICENSE.md',
                    'share/tide-terminal/vendor/zsh-autosuggestions/zsh-autosuggestions.zsh',
                    'share/tide-terminal/vendor/zsh-autosuggestions/LICENSE', 'share/doc/tide-terminal/config.example.toml']
        assert all((prefix / p).is_file() for p in required)
        assert any((prefix / 'share/man/man1').glob('tide-terminal.1*'))
        desktop = prefix / 'share/applications/tide-terminal.desktop'
        subprocess.run(['desktop-file-validate', str(desktop)], check=True)
        assert 'Exec=tide-terminal\n' in desktop.read_text() and 'X-TerminalArgExec=-e\n' in desktop.read_text()
        assert 'run.sh' not in desktop.read_text()
        results['installed_payload_and_notices'] = 'passed'
        done = subprocess.run([str(binary), '--version'], env=env, capture_output=True, timeout=5)
        assert done.returncode == 0 and b'tide-terminal' in done.stdout
        done = subprocess.run([str(prefix / 'bin/T_:'), '-s', '0.15', '--private', '--', '/bin/sleep', '10'],
                              env=env, capture_output=True, timeout=5)
        assert done.returncode != 0 and b'Runtime' in done.stderr, done
        results['installed_timer_wrapper'] = 'passed'
        done = subprocess.run([str(prefix / 'bin/A_:'), '| cat'], env=env, capture_output=True, timeout=5)
        assert done.returncode != 0 and b'needs an integrated Tide shell' in done.stderr
        results['installed_append_wrapper_reports_missing_session'] = 'passed'
        missing = subprocess.run([str(binary), '--working-directory', str(scratch / 'absent')],
                                 env=env, capture_output=True, timeout=5)
        assert missing.returncode != 0 and b'working directory' in missing.stderr
        results['invalid_working_directory_rejected'] = 'passed'
        if args.gui:
            assert env.get('DISPLAY') or env.get('WAYLAND_DISPLAY'), 'GUI checks need an isolated display'

            def launch(name, options, predicate, shell=None):
                eventlog = scratch / (name + '.jsonl')
                command = [str(binary), '--working-directory', str(scratch), '--private', '--qa',
                           '--event-log', str(eventlog)] + options
                started = time.monotonic()
                with (scratch / (name + '.log')).open('wb') as stream:
                    process = subprocess.Popen(command, env=env, stdout=stream, stderr=subprocess.STDOUT)
                    try:
                        records = []
                        checked = False
                        deadline = time.monotonic() + 15
                        while time.monotonic() < deadline:
                            if eventlog.exists():
                                records = []
                                for line in eventlog.read_text().splitlines():
                                    try:
                                        records.append(json.loads(line))
                                    except json.JSONDecodeError:
                                        pass  # The live writer may not have completed its last line.
                            if shell and not checked:
                                child = next((r['event']['pid'] for r in records if r.get('event', {}).get('event') == 'started'), None)
                                if child:
                                    settings = dict(item.split(b'=', 1) for item in Path(f'/proc/{child}/environ').read_bytes().split(b'\0') if b'=' in item)
                                    key = {'bash': b'TIDE_BLE', 'zsh': b'TIDE_ZSH_SUGGESTIONS'}.get(shell)
                                    if key:
                                        assert Path(os.fsdecode(settings[key])).is_relative_to(prefix / 'share/tide-terminal/vendor')
                                    checked = True
                            if process.poll() is not None:
                                break
                            time.sleep(.03)
                        process.wait(timeout=3)
                        records = [json.loads(line) for line in eventlog.read_text().splitlines()]
                        assert process.returncode == 0, (name, (scratch / (name + '.log')).read_text())
                        gpu = next(r['gpu'] for r in records if 'gpu' in r)
                        assert gpu['device_type'] == {'integrated': 'IntegratedGpu', 'software': 'Cpu'}[args.gpu]
                        assert predicate(records, time.monotonic() - started), (name, records)
                        if shell:
                            assert checked
                        results[name] = 'passed'
                        results['adapter'] = gpu
                    finally:
                        if process.poll() is None:
                            process.terminate()
                            process.wait(timeout=5)

            child = scratch / 'argv.json'
            values = ['two words', '', '$HOME', '$(do-not-run)', '; echo literal', 'λ', '--gpu', '-e']
            code = 'import json,os,sys;from pathlib import Path;Path(sys.argv[1]).write_text(json.dumps([os.getcwd(),sys.argv[2:]],ensure_ascii=False));print("ARGV_OK")'
            launch('desktop_exec_preserves_arguments_and_cwd', ['-e', 'python3', '-c', code, str(child), *values],
                   lambda records, elapsed: json.loads(child.read_text()) == [str(scratch), values]
                   and any(r.get('event', {}).get('event') == 'exited' and r['event']['code'] == 0 for r in records))
            launch('desktop_exec_closes_after_failure', ['-e', '/bin/sh', '-c', 'exit 7'],
                   lambda records, elapsed: any(r.get('event', {}).get('event') == 'exited' and r['event']['code'] == 7 for r in records))
            launch('desktop_hold_retains_completed_command', ['--hold', '--exit-after', '1.5s', '-e', '/bin/true'],
                   lambda records, elapsed: elapsed >= 1.5 and any(r.get('event', {}).get('event') == 'exited' for r in records))
            for shell in ['bash', 'zsh', 'fish']:
                path = shutil.which(shell)
                assert path, f'{shell} is needed to validate the package'
                launch('installed_' + shell + '_integration', ['--shell', path, '--clean', '--exit-after', '2s'],
                       lambda records, elapsed: any(r.get('event', {}).get('event') == 'prompt' for r in records), shell=shell)
        else:
            results['gui'] = 'not run'
    args.report.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
