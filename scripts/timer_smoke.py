#!/usr/bin/env python3
"""Exercise the T_: parser and the real PTY supervisor with short, harmless commands."""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--launcher', type=Path, required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    results = {}
    with tempfile.TemporaryDirectory(prefix='tide-timer-', dir=args.work_dir) as temporary:
        env = os.environ.copy()
        for key in ('TIDE_SHELL', 'TIDE_PRIVATE', 'TIDE_HISTORY'):
            env.pop(key, None)
        env.update(XDG_CONFIG_HOME=temporary, XDG_DATA_HOME=temporary)
        config = Path(temporary) / 'tide-terminal'
        config.mkdir()
        (config / 'config.toml').write_text('shell = "/bin/bash"\n')

        def run(arguments, *, environment=None, launcher=False):
            prefix = [str(args.launcher)] if launcher else [str(args.binary), 'timer']
            started = time.monotonic()
            done = subprocess.run(prefix + arguments, stdin=subprocess.DEVNULL,
                                  capture_output=True, env=environment or env, timeout=5)
            return done, time.monotonic() - started

        def events(arguments):
            done, elapsed = run(arguments + ['--private', '--json', '-grace', '100ms'])
            parsed = [json.loads(line) for line in done.stdout.splitlines()]
            assert any(e['event'] == 'exited' for e in parsed), (done.stderr, parsed)
            return done, elapsed, parsed

        def output(parsed):
            return ''.join(e['text'] for e in parsed if e['event'] == 'output')

        def triggered(parsed, reason):
            assert any(e['event'] == 'rule_triggered' and reason in e['reason']
                       for e in parsed), parsed

        ignore = "python3 -u -c 'import signal,time;signal.signal(signal.SIGINT,signal.SIG_IGN);print(\"READY\");time.sleep(20)'"
        done, elapsed, parsed = events([ignore, ':', '-s', '0.2'])
        triggered(parsed, 'Runtime')
        assert done.returncode != 0 and .15 <= elapsed < 3
        assert any(e['event'] == 'escalated' for e in parsed)
        results['seconds_colon_and_escalation'] = 'passed'

        done, elapsed, parsed = events(['sleep 20', ':', '-m', '0.004'])
        triggered(parsed, 'Runtime')
        assert .18 <= elapsed < 3 and done.returncode != 0
        results['minutes_conversion'] = 'passed'

        quiet = "python3 -u -c 'import time;[(print(\"tick\"),time.sleep(.1)) for _ in range(4)];time.sleep(20)'"
        done, elapsed, parsed = events([quiet, ':', '-idle-s', '0.2'])
        triggered(parsed, 'inactivity')
        assert .45 <= elapsed < 3
        _, _, parsed = events(['sleep 20', ':', '-idle-m', '0.004'])
        triggered(parsed, 'inactivity')
        results['idle_limits_and_reset_on_output'] = 'passed'

        _, _, parsed = events(['printf ABC123; sleep 20', ':', '-rex', '[0-9]+'])
        triggered(parsed, 'pattern')
        assert 'ABC123' in output(parsed)
        _, _, parsed = events(["printf '%s' '-rex'; sleep 20", ':', '-text', '-rex'])
        triggered(parsed, 'marker')
        results['regex_and_literal_output_rules'] = 'passed'

        done, _, parsed = events(["printf '%s\\n' 'hi \"there\"' | tr a-z A-Z", ':', '-s', '2'])
        assert done.returncode == 0 and 'HI "THERE"' in output(parsed)
        results['quoted_command_pipeline'] = 'passed'

        done, _ = run(['-s', '2', '--private', '--', 'printf', '%s\n', '-rex', ':', 'a b'])
        assert done.returncode == 0 and done.stdout == b'-rex\r\n:\r\na b\r\n', done
        results['direct_argv_preserves_arguments'] = 'passed'

        marker = Path(temporary) / 'must-not-launch'
        for flags in (['-rex', '['], ['-s', '0'], ['-s', '1', '-m', '1']):
            done, _ = run(['touch ' + shlex.quote(str(marker)), ':'] + flags)
            assert done.returncode != 0 and not marker.exists(), done
        results['invalid_rules_rejected_before_launch'] = 'passed'

        done, _, _ = events(['exit 42', ':', '-s', '2'])
        assert done.returncode == 42
        results['child_exit_code_preserved'] = 'passed'

        done, _, parsed = events(["printf 'settings:%s' {1..2}"])
        assert done.returncode == 0 and 'settings:1settings:2' in output(parsed)
        caller_env = env | {'TIDE_SHELL': '/usr/bin/fish'}
        done, _ = run(['set -l word calling_fish; printf %s $word', '--private'], environment=caller_env)
        assert done.returncode == 0 and b'calling_fish' in done.stdout
        done, _ = run(['printf %s "$BASH_VERSION"', '--shell', '/bin/bash', '--private'], environment=caller_env)
        assert done.returncode == 0 and done.stdout.strip()
        results['settings_calling_shell_and_override'] = 'passed'

        cooperative = "python3 -u -c 'import signal,time;signal.signal(signal.SIGINT,signal.SIG_IGN);time.sleep(.5);print(\"FINISHED\")'"
        done, _, parsed = events([cooperative, '-s', '0.2', '--no-force'])
        triggered(parsed, 'Runtime')
        assert done.returncode == 0 and 'FINISHED' in output(parsed)
        assert not any(e['event'] == 'escalated' for e in parsed)
        results['no_force_allows_cleanup'] = 'passed'

        done, _ = run(['printf STANDALONE_OK', ':', '-s', '2', '--private'], launcher=True)
        assert done.returncode == 0 and b'STANDALONE_OK' in done.stdout, (done.returncode, done.stdout, done.stderr)
        done, _ = run(['--help'], launcher=True)
        assert done.returncode == 0 and b'T_:' in done.stdout and b'-rex' in done.stdout
        results['standalone_launcher_and_help'] = 'passed'

        private_env = env | {'TIDE_PRIVATE': '1'}
        done, _ = run(['printf PRIVATE_RUN', ':', '-s', '2'], environment=private_env)
        assert done.returncode == 0 and not (config / 'history.sqlite3').exists()
        results['calling_shell_privacy_inherited'] = 'passed'

    args.report.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
