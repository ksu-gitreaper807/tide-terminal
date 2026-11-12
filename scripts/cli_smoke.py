#!/usr/bin/env python3
"""Test the shipped CLI, including hidden input through an outer real PTY."""
import argparse
import errno
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import termios
import time

parser=argparse.ArgumentParser()
parser.add_argument('--binary',type=Path,required=True)
parser.add_argument('--work-dir',type=Path,required=True)
parser.add_argument('--report',type=Path,required=True)
args=parser.parse_args()
results={}
with tempfile.TemporaryDirectory(prefix='tide-cli-',dir=args.work_dir) as temporary:
    env=os.environ.copy();env['XDG_DATA_HOME']=temporary
    binary=str(args.binary)
    command=[binary,'run','--private']
    started=time.monotonic()
    done=subprocess.run(command+['--json','--timeout','350ms','--grace','100ms','--','python3','-u','-c',"import signal,time;signal.signal(signal.SIGINT,signal.SIG_IGN);print('READY');time.sleep(20)"],env=env,capture_output=True,timeout=5)
    events=[json.loads(s) for s in done.stdout.splitlines()]
    assert done.returncode!=0 and time.monotonic()-started<3
    assert any(e['event']=='rule_triggered' for e in events)
    assert any(e['event']=='escalated' for e in events)
    assert any(e['event']=='exited' and e['signal'] for e in events)
    results['cli_timer_escalation_and_signal_result']='passed'

    done=subprocess.run(command+['--json','--stop-on','CLI_FINISHED','--','python3','-u','-c',"import time;print('CLI_FINISHED');time.sleep(20)"],env=env,capture_output=True,timeout=5)
    assert any(e['event']=='rule_triggered' and 'marker' in e['reason'] for e in map(json.loads,done.stdout.splitlines()))
    results['cli_output_rule']='passed'

    done=subprocess.run(command+['--','/bin/bash','-c','exit 42'],env=env,capture_output=True,timeout=5)
    assert done.returncode==42
    results['normal_exit_code_preserved']='passed'

    done=subprocess.run(command+['--','cat'],input=b'FINITE_STDIN_OK\n',env=env,capture_output=True,timeout=5)
    assert done.returncode==0 and b'FINITE_STDIN_OK' in done.stdout
    results['finite_stdin']='passed'

    marker=Path(temporary)/'must-not-launch'
    done=subprocess.run(command+['--regex','[','--','touch',str(marker)],env=env,capture_output=True,timeout=5)
    assert done.returncode!=0 and not marker.exists()
    results['invalid_rule_rejected_before_launch']='passed'

    master,slave=pty.openpty();original=termios.tcgetattr(slave)
    process=subprocess.Popen(command+['--','python3','-u','-c',"import getpass;getpass.getpass('Password: ');print('CLI_PASSWORD_ACCEPTED')"],stdin=slave,stdout=slave,stderr=slave,env=env)
    transcript=bytearray();sent=False;deadline=time.monotonic()+5
    try:
        while time.monotonic()<deadline:
            if select.select([master],[],[],.05)[0]:
                try:
                    chunk=os.read(master,8192)
                except OSError as error:
                    if error.errno==errno.EIO:break
                    raise
                transcript.extend(chunk)
            if not sent and b'Password:' in transcript:
                assert not termios.tcgetattr(slave)[3]&termios.ECHO,'Outer terminal still echoes passwords'
                os.write(master,b'cli-secret-unrecorded\n');sent=True
            if process.poll() is not None:break
        process.wait(timeout=3)
        assert sent and process.returncode==0 and b'CLI_PASSWORD_ACCEPTED' in transcript,transcript
        assert b'cli-secret-unrecorded' not in transcript,transcript
        assert termios.tcgetattr(slave)==original,'Raw terminal settings not restored'
        results['outer_pty_password_hidden_and_termios_restored']='passed'
    finally:
        if process.poll() is None:process.terminate();process.wait(timeout=3)
        os.close(master);os.close(slave)
args.report.write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps(results,indent=2))
