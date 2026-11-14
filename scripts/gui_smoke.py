#!/usr/bin/env python3
"""Exercise the actual X11 window using XTest. Needs python-xlib and Pillow.

Uses an isolated QA window, config, history, and cache. Runtime evidence comes from
session events and saved configuration, not from the test's intent to click controls.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from PIL import Image, ImageChops
from Xlib import X, XK, display
from Xlib import error as xerror
from Xlib.ext import xtest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--gpu", choices=["integrated", "discrete", "auto", "software"], default="integrated")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--kwin-input-helper",type=Path)
    parser.add_argument("--run-input-only", action="store_true")
    parser.add_argument("--focus-only", action="store_true")
    parser.add_argument("--titles-only", action="store_true")
    parser.add_argument("--chrome-only", action="store_true")
    parser.add_argument("--close-only", action="store_true")
    parser.add_argument("--logisim-binary", type=Path)
    parser.add_argument("--append-only", action="store_true")
    parser.add_argument("--scroll-selection-only", action="store_true")
    parser.add_argument("--shell", default="/bin/bash", help="Initial shell; use non-Bash shells with focused portable scenarios")
    parser.add_argument("--pager-only", action="store_true")
    parser.add_argument("--btop-binary", type=Path)
    parser.add_argument("--btop-only", action="store_true")
    parser.add_argument("--eza-binary", type=Path)
    parser.add_argument("--icons-only", action="store_true")
    args = parser.parse_args()
    if args.btop_only and not args.btop_binary:
        parser.error("--btop-only requires --btop-binary")
    if args.icons_only and not args.eza_binary:
        parser.error("--icons-only requires --eza-binary")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    args.work_dir.mkdir(parents=True, exist_ok=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix="tide-gui-", dir=args.work_dir) as temporary:
        scratch = Path(temporary)
        log = scratch / "events.jsonl"
        env = os.environ.copy()
        env.pop("WAYLAND_DISPLAY", None)
        env.update(TIDE_CONFIG=str(scratch / ".tide.toml"), XDG_CONFIG_HOME=str(scratch / "config"),
                   XDG_DATA_HOME=str(scratch / "data"),
                   XDG_CACHE_HOME=str(scratch / "cache"))
        if args.logisim_binary:
            # Keep Java preferences and user-home writes inside the test fixture.
            (scratch / "java-home").mkdir()
            (scratch / "java-prefs").mkdir()
            env["JAVA_TOOL_OPTIONS"] = f'-Djava.util.prefs.userRoot="{scratch / "java-prefs"}" -Duser.home="{scratch / "java-home"}"'
        process = subprocess.Popen([str(args.binary), "gui", "--gpu", args.gpu, "--qa", "--clean", "--shell", args.shell,
                                    "--event-log", str(log), "--screenshot",str(args.output_dir / "canvas.png")], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        d = display.Display()
        root = d.screen().root
        helper=subprocess.Popen([str(args.kwin_input_helper)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True) if args.kwin_input_helper else None
        def inject(display_connection,kind,detail=None,*,x=None,y=None):
            if not helper:
                xtest.fake_input(display_connection,kind,detail or 0,x=x or 0,y=y or 0)
                return
            if kind in (X.ButtonPress, X.ButtonRelease) and detail in (4, 5):
                if kind == X.ButtonRelease:
                    return
                line = f'axis 0 {-15 if detail == 4 else 15}'
            elif kind==X.MotionNotify:line=f'motion {x} {y}'
            elif kind in (X.KeyPress,X.KeyRelease):line=f'key {detail} {int(kind==X.KeyPress)}'
            elif kind in (X.ButtonPress,X.ButtonRelease):line=f'button {detail} {int(kind==X.ButtonPress)}'
            else:raise AssertionError(kind)
            helper.stdin.write(line+'\n');helper.stdin.flush()
            assert helper.stdout.readline().strip()=='OK','Isolated input helper stopped'

        def records():
            if not log.exists():
                return []
            result = []
            for line in log.read_text().splitlines():
                try:
                    result.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
            return result

        def wait(predicate, description, timeout=15):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                try:
                    value = predicate()
                except xerror.BadWindow:
                    value=None
                if value:
                    return value
                if process.poll() is not None:
                    raise AssertionError(f"Window exited during {description}: {process.communicate()[1].decode()}")
                time.sleep(.05)
            raise AssertionError(f"Timeout: {description}")

        def event_after(index, predicate):
            return next((r for r in records()[index:] if "event" in r and predicate(r["event"])), None)

        def output_since(index):
            return "".join(r["event"]["text"] for r in records()[index:]
                           if r.get("event", {}).get("event") == "output")

        def find_window():
            def visit(window):
                try:
                    if window.get_wm_name() == "Tide Terminal QA":
                        return window
                    for child in window.query_tree().children:
                        found = visit(child)
                        if found:
                            return found
                except Exception:
                    pass
                return None
            return visit(root)

        def key(name, modifiers=()):
            codes = [d.keysym_to_keycode(XK.string_to_keysym(m)) for m in modifiers]
            for code in codes:
                inject(d, X.KeyPress, code)
            code = d.keysym_to_keycode(XK.string_to_keysym(name))
            inject(d, X.KeyPress, code)
            inject(d, X.KeyRelease, code)
            for code in reversed(codes):
                inject(d, X.KeyRelease, code)
            d.sync()
            time.sleep(.07)

        def text(value):
            for char in value:
                if char == "\n":
                    key("Return")
                    continue
                symbol = ord(char)
                code = d.keysym_to_keycode(symbol)
                assert code, f"No X11 keycode for {char!r}"
                mapping = d.get_keyboard_mapping(code, 1)[0]
                shift = next((i % 2 for i, sym in enumerate(mapping) if sym == symbol), 0)
                if shift:
                    inject(d, X.KeyPress, d.keysym_to_keycode(XK.XK_Shift_L))
                inject(d, X.KeyPress, code)
                inject(d, X.KeyRelease, code)
                if shift:
                    inject(d, X.KeyRelease, d.keysym_to_keycode(XK.XK_Shift_L))
                time.sleep(.002)
            d.sync()
            time.sleep(.1)

        window = wait(find_window, "find QA window")
        wait(lambda: window.get_attributes().map_state == X.IsViewable, "QA window mapped")
        time.sleep(.3)
        window.set_input_focus(X.RevertToParent, X.CurrentTime)
        d.sync()
        assert d.get_input_focus().focus.id == window.id

        def ui_state():
            return next((r for r in reversed(records()) if "ui" in r), {})

        def click(name, x_fraction=.5, y_fraction=.5):
            previous = None
            stable_since = time.monotonic()
            def settled_control():
                nonlocal previous, stable_since
                state = ui_state()
                rect = state.get("ui", {}).get(name)
                if rect is None:
                    return None
                if rect != previous:
                    previous = rect
                    stable_since = time.monotonic()
                return state if time.monotonic() - stable_since >= .18 else None
            # Modal Areas settle their size/position over multiple passes.
            # Clicking an earlier layout can hit a different control.
            state = wait(settled_control, f"settled control {name}")
            x1, y1, x2, y2 = state["ui"][name]
            origin = root.translate_coords(window, 0, 0)
            x = origin.x + round((x1 + (x2-x1)*x_fraction)*state["scale"])
            y = origin.y + round((y1 + (y2-y1)*y_fraction)*state["scale"])
            inject(d, X.MotionNotify, x=x, y=y)
            inject(d, X.ButtonPress, 1)
            d.sync()
            time.sleep(.08)
            inject(d, X.ButtonRelease, 1)
            d.sync()
            time.sleep(.2)

        def field(name, value):
            click(name)
            key("a", ("Control_L",))
            key("BackSpace")
            text(value)

        def frame():
            d.sync()
            geometry = window.get_geometry()
            raw = window.get_image(0, 0, geometry.width, geometry.height, X.ZPixmap, 0xffffffff)
            return Image.frombytes("RGB", (geometry.width, geometry.height), raw.data, "raw", "BGRX")

        def native_frame():
            # XGetImage can expose an old Vulkan/Xwayland backing pixmap.
            # Request the application's actual freshly rendered GPU frame.
            canvas = args.output_dir / "canvas.png"
            old = canvas.stat().st_mtime_ns if canvas.exists() else 0
            key("s", ("Control_L", "Shift_L"))
            requested_at = time.monotonic()
            def complete():
                nonlocal requested_at
                if not canvas.exists() or canvas.stat().st_mtime_ns == old:
                    # GPU readback is asynchronous. An exited terminal may be
                    # idle, so another capture request also supplies a repaint.
                    if time.monotonic() - requested_at > .5:
                        key("s", ("Control_L", "Shift_L"))
                        requested_at = time.monotonic()
                    return None
                try:
                    with Image.open(canvas) as image:
                        image.load()
                        return image.copy()
                except (OSError, ValueError):
                    return None
            return wait(complete, "fresh native GPU capture")

        def screenshot(name):
            native_frame().save(args.output_dir / name)

        def start_run(command, timeout="", marker="", shortcut=False):
            if shortcut:
                key("r", ("Control_L", "Shift_L"))
            else:
                click("run")
            field("command", command)
            wait(lambda: ui_state().get("run_command") == command, "complete command text reaches Run editor")
            field("timeout", timeout)
            field("idle", "")
            field("marker", marker)
            field("pattern", "")
            index = len(records())
            click("start_run")
            wait(lambda: event_after(index, lambda e: e["event"] == "started" and e["managed"]), "managed process started")
            return index

        def close_fixture_tab():
            # Teardown owns these tabs and explicitly accepts the new close guard.
            # A shell can still be starting, or have a background helper, even
            # after the test command itself has completed.
            wait(lambda: 'close_confirm' not in ui_state().get('ui', {}), 'previous close dialog clears')
            previous = len(ui_state()['tabs'])
            key('w', ('Control_L', 'Shift_L'))
            outcome = wait(lambda: 'closed' if len(ui_state().get('tabs', [])) < previous
                           else ('confirm' if 'close_confirm' in ui_state().get('ui', {}) else None),
                           'fixture tab closes or requests confirmation')
            if outcome == 'confirm':
                click('close_confirm')
            wait(lambda: len(ui_state()['tabs']) < previous, 'fixture tab closes after explicit decision')

        def check_terminal_focus():
            import shlex
            expected = b'\t\x1b[Z\x1b[D\x1b[C\x1b[A\x1b[Babc'
            script = ("import os,tty;tty.setraw(0);print('FOCUS_READY',flush=True);"
                      f"data=b''.join(os.read(0,1) for _ in range({len(expected)}));"
                      "print('FOCUS_BYTES='+data.hex(),flush=True)")
            index = start_run('python3 -u -c ' + shlex.quote(script), shortcut=True)
            wait(lambda: 'FOCUS_READY' in output_since(index), 'raw focus test ready')
            click('terminal', .2, .2)
            key('Tab')
            key('Tab', ('Shift_L',))
            for arrow in ('Left', 'Right', 'Up', 'Down'):
                key(arrow)
            text('abc')
            wait(lambda: 'FOCUS_BYTES='+expected.hex() in output_since(index),
                 'Tab, Shift+Tab and arrows reach the child without needing Escape', timeout=5)
            wait(lambda: event_after(index, lambda e:e['event']=='exited' and e['code']==0), 'focus reader exits')
            results['gui_terminal_navigation_keeps_keyboard_focus'] = 'passed'
            click('tab_0')
            index = len(records())
            text("printf 'FOCUS_TAB_CLICK_OK\\n'\n")
            wait(lambda: event_after(index, lambda e:e['event']=='command_ended' and e['code']==0),
                 'typing after clicking a tab works without Escape')
            assert 'FOCUS_TAB_CLICK_OK\r\n' in output_since(index)
            results['gui_tab_click_restores_terminal_typing'] = 'passed'
            click('run')
            field('command', "printf 'DIALOG_MUST_NOT_RUN'")
            key('Tab')
            text('0.5')
            wait(lambda: ui_state().get('run_timeout') == '0.5', 'Tab moves between dialog fields')
            assert ui_state()['run_command'] == "printf 'DIALOG_MUST_NOT_RUN'"
            results['gui_modal_keeps_text_and_tab_navigation'] = 'passed'
            click('run_cancel')
            wait(lambda: 'command' not in ui_state().get('ui', {}), 'mouse cancel closes dialog')
            index = len(records())
            text("printf 'FOCUS_CANCEL_OK\\n'\n")
            wait(lambda: event_after(index, lambda e:e['event']=='command_ended' and e['code']==0),
                 'typing resumes after cancel without terminal click or Escape')
            assert not event_after(index, lambda e:e['event']=='command_started' and 'DIALOG_MUST_NOT_RUN' in e['command'])
            results['gui_modal_cancel_restores_terminal_typing'] = 'passed'
            click('tab_1')
            key('w', ('Control_L', 'Shift_L'))

        def check_chrome():
            wait(lambda: 'tab_strip' not in ui_state().get('ui', {}), 'single terminal hides tab strip')
            single = ui_state()['ui']['terminal']
            assert 0 <= single[1] <= 8, f'single terminal reserves an empty header: {single}'
            def actions_fit():
                ui = ui_state().get('ui', {})
                if 'action_panel' not in ui:
                    return False
                a, b, c, d = ui['action_panel']
                return all(name in ui and a <= ui[name][0] < ui[name][2] <= c
                           and b <= ui[name][1] < ui[name][3] <= d
                           for name in ('new_tab', 'run', 'append', 'history', 'settings'))
            wait(actions_fit, 'rounded floating controls contain every action')
            screenshot('single-terminal-glass.png')
            index = len(records())
            click('terminal', .3, .3)
            click('new_tab')
            wait(lambda: len(ui_state().get('tabs', [])) == 2 and 'tab_strip' in ui_state().get('ui', {}), 'multiple terminals reveal tab strip')
            wait(lambda: ui_state().get('pane_transition') == 1, 'new terminal transition settles')
            wait(lambda: ui_state()['ui']['terminal'][1] >= single[1] + 47,
                 'terminal makes room for the tab bar')
            multiple = ui_state()['ui']['terminal']
            assert abs(multiple[1] - single[1] - 48) <= 1, (single, multiple)
            assert any(single[1] + .5 < r['ui']['terminal'][1] < multiple[1] - .5
                       for r in records()[index:] if 'terminal' in r.get('ui', {})), \
                'opening tabs did not smoothly expand the top margin'
            assert any(0 < r.get('pane_transition', 1) < 1 for r in records()[index:] if 'ui' in r)
            wait(actions_fit, 'floating actions remain separate from tabs')
            screenshot('multiple-terminal-glass.png')
            index = len(records())
            before = ui_state()['ui']['terminal']
            click('tab_0')
            wait(lambda: ui_state()['active_tab'] == 0 and ui_state().get('pane_transition') == 1, 'tab switch settles')
            assert any(0 < r.get('pane_transition', 1) < 1 for r in records()[index:] if 'ui' in r)
            assert ui_state()['ui']['terminal'] == before, 'tab animations moved the terminal input grid'
            results['gui_tab_open_and_switch_transitions'] = 'passed'
            click('tab_1')
            index = len(records())
            close_fixture_tab()
            wait(lambda: 'tab_strip' not in ui_state().get('ui', {}), 'tab strip hides after returning to one terminal')
            wait(lambda: abs(ui_state()['ui']['terminal'][1] - single[1]) <= 1,
                 'single terminal reclaims the tab bar space')
            assert any(single[1] + .5 < r['ui']['terminal'][1] < multiple[1] - .5
                       for r in records()[index:] if 'terminal' in r.get('ui', {})), \
                'closing the second tab did not smoothly collapse the top margin'
            screenshot('single-terminal-margin-restored.png')
            results['gui_terminal_top_margin_tracks_tab_bar'] = 'passed'
            results['gui_single_terminal_hides_tab_strip'] = 'passed'
            results['gui_floating_actions_are_reachable'] = 'passed'

            # A managed run can become the only tab after its source shell closes.
            # Its Stop button must remain clickable beside the floating controls.
            index = start_run('sleep 20')
            click('tab_0')
            close_fixture_tab()
            def managed_controls_fit():
                ui = ui_state().get('ui', {})
                if 'tab_strip' in ui or 'stop_run' not in ui or 'action_panel' not in ui:
                    return False
                stop, actions = ui['stop_run'], ui['action_panel']
                return stop[2] + 8 <= actions[0] and stop[1] < actions[3]
            wait(managed_controls_fit, 'single managed terminal keeps Stop beside floating actions')
            geometry = window.get_geometry()
            window.configure(width=640, height=420)
            d.sync()
            wait(lambda: window.get_geometry().width == 640 and window.get_geometry().height == 420,
                 'compact single managed terminal')
            wait(lambda: managed_controls_fit() and actions_fit(), 'compact managed controls remain reachable')
            screenshot('single-managed-terminal-controls.png')
            click('stop_run')
            wait(lambda: event_after(index, lambda e: e['event'] == 'exited'), 'Stop interrupts the single managed run')
            results['gui_single_managed_terminal_stop_stays_reachable'] = 'passed'
            close_fixture_tab()
            window.configure(width=geometry.width, height=geometry.height)
            d.sync()
            index = len(records())
            click('new_tab')
            wait(lambda: event_after(index, lambda e: e['event'] == 'prompt'), 'fresh single shell after managed layout test')

        def activate_terminal_window():
            # A Java application can map/raise a window after its launcher returns.
            # Activate only Tide on this isolated test display before targeting it.
            from Xlib.protocol import event
            root.send_event(event.ClientMessage(window=window, client_type=d.intern_atom('_NET_ACTIVE_WINDOW'),
                data=(32, [2, X.CurrentTime, 0, 0, 0])),
                event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
            window.configure(stack_mode=X.Above)
            window.set_input_focus(X.RevertToParent, X.CurrentTime)
            d.sync()
            wait(lambda: getattr(d.get_input_focus().focus, 'id', None) == window.id, 'activate isolated Tide window')
            # XSetInputFocus alone does not establish KWin's Wayland seat focus.
            # Activate through a harmless click before targeting a control.
            click('terminal', .95, .95)
            active_atom = d.intern_atom('_NET_ACTIVE_WINDOW')
            if root.get_full_property(active_atom, X.AnyPropertyType):
                wait(lambda: int(root.get_full_property(active_atom, X.AnyPropertyType).value[0]) == window.id,
                     'compositor activates Tide, not just X11 input focus')

        def request_window_close():
            from Xlib.protocol import event
            message = event.ClientMessage(window=window, client_type=d.intern_atom("WM_PROTOCOLS"),
                data=(32, [d.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]))
            window.send_event(message, event_mask=0)
            d.flush()

        def check_close_protection():
            import shlex
            pid_file = scratch / 'close-job.pid'
            code = f'import os,signal,time,pathlib; signal.signal(signal.SIGHUP,signal.SIG_IGN); pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid())); time.sleep(60)'
            start_run('python3 -u -c ' + shlex.quote(code))
            wait(pid_file.exists, 'close fixture starts')
            pid = int(pid_file.read_text())
            count = len(ui_state()['tabs'])
            key('w', ('Control_L', 'Shift_L'))
            wait(lambda: 'close_confirm' in ui_state().get('ui', {}), 'busy tab requires confirmation')
            click('terminal', .05, .05)  # Outside the warning: never an implicit choice.
            wait(lambda: 'close_confirm' in ui_state().get('ui', {}), 'close warning persists until an explicit decision', timeout=3)
            results['gui_close_confirmation_survives_outside_click'] = 'passed'
            click('close_cancel')
            wait(lambda: 'close_confirm' not in ui_state().get('ui', {}), 'keep open cancels close')
            assert len(ui_state()['tabs']) == count and Path(f'/proc/{pid}').exists()
            click('tab_close_' + str(ui_state()['active_tab']))
            wait(lambda: 'close_confirm' in ui_state().get('ui', {}), 'first click after cancel reopens quiet terminal warning')
            click('close_cancel')
            results['gui_quiet_terminal_close_reopens_after_cancel'] = 'passed'
            request_window_close()
            wait(lambda: 'close_confirm' in ui_state().get('ui', {}), 'window close requires confirmation')
            screenshot('close-protection.png')
            key('Escape')
            wait(lambda: 'close_confirm' not in ui_state().get('ui', {}), 'Escape cancels window close')
            assert process.poll() is None and Path(f'/proc/{pid}').exists()
            close_fixture_tab()
            wait(lambda: len(ui_state()['tabs']) == count - 1, 'confirmed close removes only targeted tab')
            wait(lambda: not Path(f'/proc/{pid}').exists(), 'confirmed close leaves no live or zombie fixture')
            results['gui_busy_tab_close_can_be_cancelled'] = 'passed'
            results['gui_window_close_can_be_cancelled'] = 'passed'
            results['gui_confirmed_close_cleans_resistant_process'] = 'passed'
            # Detection must still warn when the prompt has returned after &.
            click('terminal')
            pid_file.unlink()
            text('python3 -u -c ' + shlex.quote(code) + ' &\n')
            wait(pid_file.exists, 'background close fixture starts')
            pid = int(pid_file.read_text())
            request_window_close()
            wait(lambda: 'close_confirm' in ui_state().get('ui', {}), 'background job protects window')
            click('close_cancel')
            assert Path(f'/proc/{pid}').exists()
            # Finish this fixture through the shell so subsequent tests keep one idle tab.
            text(f'kill -KILL {pid}; wait\n')
            wait(lambda: not Path(f'/proc/{pid}').exists(), 'background fixture reaped')
            results['gui_background_job_protects_idle_shell'] = 'passed'
            for name, binary, arguments in [('btop', args.btop_binary, btop_arguments() if args.btop_binary else []), ('logisim', args.logisim_binary, [])]:
                if not binary:
                    continue
                index = start_run(shlex.join([str(binary), *arguments]))
                started = wait(lambda: event_after(index, lambda e:e['event']=='started'), f'{name} starts')
                root_pid = started['event']['pid']
                time.sleep(1.5)
                # Record every process in this fixture's terminal session, including Java.
                members = []
                for stat in Path('/proc').glob('[0-9]*/stat'):
                    try:
                        fields = stat.read_text().rsplit(') ', 1)[1].split()
                        if int(fields[3]) == root_pid:
                            members.append(int(stat.parent.name))
                    except (OSError, ValueError, IndexError):
                        pass
                assert members, f'{name} exited before close test'
                if name == 'logisim':
                    def java_windows():
                        clients = root.get_full_property(d.intern_atom('_NET_CLIENT_LIST'), X.AnyPropertyType)
                        found = []
                        for identifier in clients.value if clients else []:
                            client = d.create_resource_object('window', int(identifier))
                            prop = client.get_full_property(d.intern_atom('_NET_WM_PID'), X.AnyPropertyType)
                            if prop and int(prop.value[0]) in members and client.get_attributes().map_state == X.IsViewable:
                                found.append(client)
                        return found
                    # Wait for the actual GUI, not merely its JVM launcher. Minimize
                    # only this fixture's verified windows so they cannot cover Tide.
                    clients = wait(java_windows, 'Logisim Java window appears')
                    from Xlib.protocol import event
                    for client in clients:
                        root.send_event(event.ClientMessage(window=client, client_type=d.intern_atom('WM_CHANGE_STATE'),
                            data=(32, [3, 0, 0, 0, 0])),
                            event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
                    d.flush()
                    wait(lambda: all(client.get_attributes().map_state != X.IsViewable for client in clients), 'isolated Logisim windows minimized')
                activate_terminal_window()
                click('tab_close_' + str(ui_state()['active_tab']))
                wait(lambda: 'close_confirm' in ui_state().get('ui', {}), f'{name} requires confirmation')
                click('close_cancel')
                assert any(Path(f'/proc/{p}').exists() for p in members)
                activate_terminal_window()
                click('tab_close_' + str(ui_state()['active_tab']))
                click('close_confirm')
                wait(lambda: all(not Path(f'/proc/{p}').exists() for p in members), f'{name}: no surviving or zombie processes')
                results[f'gui_{name}_close_guard_and_process_cleanup'] = 'passed'

        def check_window_shutdown():
            index = start_run('sleep 60')
            started = wait(lambda: event_after(index, lambda e:e['event']=='started'), 'window shutdown fixture starts')
            pid = started['event']['pid']
            request_window_close()
            click('close_confirm')
            process.wait(timeout=8)
            assert not Path(f'/proc/{pid}').exists(), 'confirmed window close did not reap direct child'
            results['gui_confirmed_window_close_reaps_session'] = 'passed'

        def check_tab_titles():
            first = ui_state()['active_tab']
            command = "printf 'TITLE_ONE\\n'"
            index = len(records())
            text(command + '\n')
            wait(lambda: event_after(index, lambda e:e['event']=='command_ended' and e['code']==0), 'first title command completes')
            wait(lambda: ui_state().get('tabs', [])[first] == command, 'tab title retains the full last command')
            results['gui_tab_title_tracks_completed_command'] = 'passed'
            index = len(records())
            key('t', ('Control_L', 'Shift_L'))
            wait(lambda: event_after(index, lambda e:e['event']=='prompt'), 'new title test shell ready')
            second = ui_state()['active_tab']
            assert second != first
            assert ui_state()['tabs'][second] == 'bash'
            index = len(records())
            text('false\n')
            wait(lambda: event_after(index, lambda e:e['event']=='command_ended' and e['code']==1), 'failed command completes')
            wait(lambda: ui_state()['tabs'][second] == 'false', 'failed command also updates the title')
            assert ui_state()['tabs'][first] == command
            click(f'tab_{first}')
            assert ui_state()['tabs'][first] == command
            results['gui_tab_titles_are_isolated_and_preserve_failed_commands'] = 'passed'
            click(f'tab_{second}')
            key('w', ('Control_L', 'Shift_L'))
            index = start_run('sleep 30', timeout='0.2', shortcut=True)
            managed = ui_state()['active_tab']
            wait(lambda: ui_state()['tabs'][managed] == 'sleep 30', 'managed tab shows its full command')
            wait(lambda: event_after(index, lambda e:e['event']=='exited'), 'managed title test stops')
            assert ui_state()['tabs'][managed] == 'sleep 30'
            assert ui_state()['tabs'][first] == command
            results['gui_managed_tab_title_persists_after_exit'] = 'passed'
            screenshot('command-titles.png')
            key('w', ('Control_L', 'Shift_L'))

        def btop_arguments():
            # Keep process command lines out of diagnostic logs and screenshots.
            config_dir = scratch / "config/btop"
            config_dir.mkdir(parents=True, exist_ok=True)
            (config_dir / "btop.conf").write_text('shown_boxes = "cpu mem"\ngraph_symbol = "braille"\nupdate_ms = 500\n')
            help_text = subprocess.run([str(args.btop_binary), "--help"], env=env, capture_output=True, text=True, timeout=5, check=True).stdout
            utf_flag = next((flag for flag in ("--force-utf", "--utf-force") if flag in help_text), None)
            assert utf_flag, "btop --help advertises no supported force-UTF option"
            return [utf_flag]

        def check_btop():
            import shlex
            index = start_run(shlex.join([str(args.btop_binary), *btop_arguments()]), shortcut=args.btop_only)
            wait(lambda: event_after(index, lambda e: e["event"] == "output" and any('\u2800' <= c <= '\u28ff' for c in e["text"])), "real btop emits Braille plot glyphs")
            time.sleep(2)
            screenshot("btop-utf.png")
            click("terminal", .5, .5)
            key("q")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "btop exits normally after q")
            results["gui_btop_force_utf_braille_graphs"] = "passed"

        def check_icons():
            import shlex
            fixture = scratch / "icon-files"
            fixture.mkdir()
            for name in [".config", "Documents", "Downloads", "src", "node_modules"]:
                (fixture / name).mkdir()
            for name in ["main.rs", "main.py", "index.ts", "index.html", "README.md", "Cargo.toml", "package.json", "Dockerfile", "notes.txt", "image.png", "archive.zip", "song.mp3", "video.mp4", "report.pdf"]:
                (fixture / name).touch()
            index = start_run(shlex.quote(str(args.eza_binary)) + " --icons=always --color=always -la " + shlex.quote(str(fixture)))
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "real eza icon listing completes")
            output = "".join(r["event"]["text"] for r in records()[index:] if r.get("event", {}).get("event") == "output")
            assert any('\ue000' <= c <= '\uf8ff' for c in output), "eza did not emit icon code points"
            assert any(0xf0000 <= ord(c) <= 0xffffd for c in output), "eza did not emit Material Design icons"
            screenshot("eza-icons.png")
            results["gui_eza_file_and_folder_icons"] = "passed"
            gallery = Path(__file__).resolve().parent.parent / "tests/fixtures/glyph-gallery.txt"
            index = start_run("cat " + shlex.quote(str(gallery)))
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "icon family gallery completes")
            screenshot("symbol-gallery.png")
            results["gui_nerd_font_family_gallery"] = "passed"

        def check_pager():
            # A real pager opens ttyname(2) on some less versions. Wheel input
            # and terminal query replies must reach that input terminal.
            import shlex
            pager_file = scratch / "pager-lines.txt"
            pager_file.write_text("".join(f"PAGER_LINE_{n:04d}\n" for n in range(1, 240)))
            tabs_before = sum(name.startswith("tab_") and name[4:].isdigit() for name in ui_state().get("ui", {}))
            index = start_run("less -R " + shlex.quote(str(pager_file)))
            wait(lambda: event_after(index, lambda e: e["event"] == "output" and "PAGER_LINE_0001" in e["text"]), "real pager first page")
            click("terminal", .5, .4)
            before = native_frame().convert("RGB")
            before.save(args.output_dir / "pager-before.png")
            moved_index = len(records())
            for _ in range(3):
                inject(d, X.ButtonPress, 5)
                inject(d, X.ButtonRelease, 5)
                d.sync()
                time.sleep(.12)
            wait(lambda: event_after(moved_index, lambda e: e["event"] == "output" and "PAGER_LINE_" in e["text"]), "mouse wheel scrolls pager content")
            after = native_frame().convert("RGB")
            after.save(args.output_dir / "pager-after.png")
            from PIL import ImageChops
            crop = (20, 100, before.width - 20, before.height - 80)
            assert ImageChops.difference(before.crop(crop), after.crop(crop)).getbbox(), "Pager content did not move"
            output = "".join(r["event"]["text"] for r in records()[index:] if r.get("event", {}).get("event") == "output")
            assert "^[]10;rgb:" not in output and "^[[?6c" not in output
            key("q")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "pager quits through GUI keyboard")
            key("w", ("Control_L", "Shift_L"))
            wait(lambda: sum(name.startswith("tab_") and name[4:].isdigit() for name in ui_state().get("ui", {})) == tabs_before, "finished pager tab closes")
            results["gui_pager_wheel_scroll_and_quit"] = "passed"

        def check_scrollback_selection():
            import shlex
            def command(value):
                index=len(records());click("terminal",.1,.4);text(value+"\n")
                wait(lambda:event_after(index,lambda e:e["event"]=="command_ended" and e["code"]==0),"selection fixture command completes")
            def cell_position(row,col):
                state=ui_state();left,top,right,bottom=state['ui']['terminal_cell']
                origin=root.translate_coords(window,0,0)
                return (origin.x+round((left+(col+.5)*(right-left))*state['scale']),
                        origin.y+round((top+(row+.5)*(bottom-top))*state['scale']))
            def drag(a,b):
                inject(d,X.MotionNotify,x=cell_position(*a)[0],y=cell_position(*a)[1]);d.sync();time.sleep(.08)
                inject(d,X.ButtonPress,1);d.sync();time.sleep(.1)
                # A single long motion reproduces a fast real drag: the drag-start
                # frame sees the endpoint, but selection must retain the press origin.
                inject(d,X.MotionNotify,x=cell_position(*b)[0],y=cell_position(*b)[1]);d.sync();time.sleep(.15)
                inject(d,X.ButtonRelease,1);d.sync();time.sleep(.15)
            def copied(expected,name):
                key('c',('Control_L','Shift_L'))
                target=scratch/name
                text("printf '%s' '");key('v',('Control_L','Shift_L'));text("' > "+shlex.quote(name)+"\n")
                wait(target.exists,'selected text pasted and saved')
                assert target.read_text()==expected,(expected,target.read_text())

            command('cd '+shlex.quote(str(scratch)))
            # Clear the visible screen before saved history: clearing the screen
            # may itself move previously visible rows into the scrollback.
            clear="printf '\\033[H\\033[2J\\033[3J"
            sample='SELECT_THIS_TEXT'
            for start,end,name in [((0,0),(0,len(sample)-1),'forward-selection'),((0,len(sample)-1),(0,0),'reverse-selection')]:
                command(clear+sample+"\\r\\n'")
                assert not ui_state()['terminal_view']['mouse_reporting'],'clean shell unexpectedly captured mouse input'
                drag(start,end)
                selection=ui_state()['terminal_view']['selection']
                assert selection==[list(start),list(end)],selection
                screenshot(name+'.png');copied(sample,name+'.txt')
            results['gui_fast_forward_reverse_drag_and_clipboard']='passed'

            fixture=scratch/'scroll-lines.txt'
            fixture.write_text(''.join(f'SCROLL_LINE_{n:04d}\n' for n in range(120)))
            command(clear+"'; cat scroll-lines.txt")
            wait(lambda:ui_state()['terminal_view']['offset']==0,'new output starts at live prompt')
            x,y=cell_position(10,8);inject(d,X.MotionNotify,x=x,y=y);d.sync()
            before=native_frame().convert('RGB')
            for _ in range(3):
                inject(d,X.ButtonPress,4);inject(d,X.ButtonRelease,4);d.sync();time.sleep(.2)
            wait(lambda:ui_state()['terminal_view']['offset']>0,'small wheel movements scroll shell history',timeout=4)
            after=native_frame().convert('RGB')
            assert ImageChops.difference(before,after).getbbox(),'scrollback did not change rendered content'
            results['gui_small_wheel_movements_scroll_shell_history']='passed'
            for _ in range(140):
                inject(d,X.ButtonPress,4);inject(d,X.ButtonRelease,4);d.sync();time.sleep(.01)
            time.sleep(.5)
            sample='SCROLL_LINE_0000';drag((0,0),(0,len(sample)-1))
            screenshot('scrollback-selection.png')
            copied(sample,'scrollback-copy.txt')
            results['gui_select_and_copy_scrollback']='passed'

        def check_appending():
            import shlex
            def choose(mode):
                click("append_mode")
                click("append_" + mode)
            def append(mode, target, shortcut=False):
                if shortcut:
                    key("a", ("Control_L", "Shift_L"))
                else:
                    click("append")
                wait(lambda: "append_target" in ui_state().get("ui", {}), "append dialog ready")
                choose(mode)
                field("append_target", target)
                index = len(records())
                click("append_submit")
                try:
                    wait(lambda: "append_target" not in ui_state().get("ui",{}), "append dialog closes after submission", timeout=3)
                except AssertionError:
                    screenshot("append-submit-failed.png")
                    raise
                return index
            def completed(index, code=0):
                return wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == code), "append stage completes")
            def stage_text(index):
                return "".join(r["event"]["text"] for r in records()[index:] if r.get("event",{}).get("event") == "output")
            index=start_run("printf 'alpha\\nbeta\\n'")
            completed(index)
            key("a", ("Control_L", "Shift_L"))
            wait(lambda: "append_target" in ui_state().get("ui",{}), "append shortcut opens dialog")
            # The new field must accept text without a mouse click, like Run.
            text("grep alpha")
            choose("pipe")
            screenshot("append-dialog.png")
            index=len(records());click("append_submit");completed(index)
            assert stage_text(index)=="alpha\r\n",stage_text(index)
            results["gui_append_captured_pipe_and_shortcut_typing"]="passed"

            index=start_run("read reply; printf SOURCE_DONE")
            appended=append("and","printf AFTER_SUCCESS")
            time.sleep(.2)
            assert not event_after(appended,lambda e:e["event"]=="started"),"Follow-up ran before the source finished"
            click("terminal");text("continue\n")
            wait(lambda:event_after(appended,lambda e:e["event"]=="output" and "AFTER_SUCCESS" in e["text"]),"queued success command runs")
            completed(appended)
            results["gui_append_waits_for_success"]="passed"

            index=start_run("false");completed(index,1)
            appended=append("and","printf MUST_NOT_RUN")
            time.sleep(.4)
            assert not event_after(appended,lambda e:e["event"]=="started"),"Failed source incorrectly ran && action"
            results["gui_append_skips_failed_source"]="passed"

            target=scratch/"appended output.txt";target.write_text("old\n")
            index=start_run("printf 'new\\n'");completed(index)
            append("file",str(target))
            screenshot("append-file-submitted.png")
            wait(lambda:target.read_text()=="old\nnew\n","captured text appended without truncating existing file")
            results["gui_append_redirect_preserves_file"]="passed"

            forbidden=scratch/"cancelled-followup"
            index=start_run("read reply; printf CANCEL_SOURCE_DONE")
            appended=append("and","touch "+shlex.quote(str(forbidden)))
            click("append");click("append_cancel_queued");key("Escape")
            click("terminal");text("continue\n");completed(index)
            time.sleep(.2);assert not forbidden.exists()
            assert not event_after(appended,lambda e:e["event"]=="started")
            results["gui_append_queue_cancellation"]="passed"

            forbidden=scratch/"closed-source-followup"
            index=start_run("read reply; printf CLOSED_SOURCE_DONE")
            appended=append("and","touch "+shlex.quote(str(forbidden)))
            key("w",("Control_L","Shift_L"))
            click("close_confirm")
            time.sleep(.4);assert not forbidden.exists()
            assert not event_after(appended,lambda e:e["event"]=="started")
            results["gui_append_source_close_cancels_queue"]="passed"
            while len(ui_state()['tabs']) > 1:
                close_fixture_tab()

            # Exercise A_: from the real prompt, not from the Run/Append dialogs.
            def prompt_command(command):
                index = len(records())
                click("terminal"); text(command + "\n")
                wait(lambda: event_after(index, lambda e: e["event"] == "command_ended" and e["code"] == 0), "CLI append command completes")
            counter=scratch/"cli-source-counter"
            prompt_command("printf x >> " + shlex.quote(str(counter)) + "; printf 'Hello\\nbeta\\n'; printf 'error\\n' >&2")
            filtered=scratch/"cli-filtered"
            prompt_command("A_: " + shlex.quote("| grep -i hello > " + shlex.quote(str(filtered))))
            assert filtered.read_text()=="Hello\n"
            assert counter.read_text()=="x"
            results["gui_cli_append_pipe_from_completed_prompt"]="passed"
            saved=scratch/"cli-saved output.txt"; saved.write_text("old\n")
            prompt_command("A_: " + shlex.quote(">> " + shlex.quote(str(saved))))
            assert saved.read_text().startswith("old\n")
            assert sorted(saved.read_text().splitlines()[1:])==["Hello","beta","error"]
            screenshot("append-cli.png")
            results["gui_cli_append_file_reuses_original_capture"]="passed"
            stdout_file=scratch/"cli-stdout.txt"
            stderr_file=scratch/"cli-stderr.txt"
            prompt_command("A_: " + shlex.quote("1>> " + shlex.quote(str(stdout_file))))
            prompt_command("A_: " + shlex.quote("2>> " + shlex.quote(str(stderr_file))))
            assert stdout_file.read_text()=="Hello\nbeta\n"
            assert stderr_file.read_text()=="error\n"
            results["gui_cli_append_selects_stdout_file"]="passed"
            results["gui_cli_append_selects_stderr_file"]="passed"
            screenshot("append-cli-streams.png")

            nested=scratch/"nested-filtered.txt"
            action="&& printf 'kushal\\nother\\n' | A_: " + shlex.quote("| grep k > " + shlex.quote(str(nested)))
            prompt_command("A_: " + shlex.quote(action))
            assert nested.read_text()=="kushal\n"
            assert counter.read_text()=="x"
            results["gui_cli_nested_append_uses_immediate_pipeline_input"]="passed"

            timed=scratch/"nested-timed.txt"
            body="printf 'timed\\n' | A_: " + shlex.quote(">> " + shlex.quote(str(timed)))
            prompt_command("A_: " + shlex.quote("&& T_: " + shlex.quote(body) + " : -s 2"))
            assert timed.read_text()=="timed\n"
            prompt_command("A_: " + shlex.quote("1>> " + shlex.quote(str(stdout_file))))
            assert stdout_file.read_text()=="Hello\nbeta\nHello\nbeta\n"
            results["gui_cli_nested_timer_preserves_original_capture"]="passed"

        try:
            wait(lambda: event_after(0, lambda e: e["event"] == "prompt"), "initial shell prompt")
            gpu=wait(lambda:next((r['gpu'] for r in records() if 'gpu' in r),None),'actual GPU adapter')
            expected = {"integrated": "IntegratedGpu", "discrete": "DiscreteGpu", "software": "Cpu"}.get(args.gpu)
            if expected:
                assert gpu['device_type'] == expected, gpu
            results['integrated_gpu_rendering' if args.gpu == 'integrated' else 'explicit_renderer'] = gpu
            # Verify the icon that winit actually publishes to the window manager.
            # This catches falling back to the framework's generic window icon.
            icon = window.get_full_property(d.intern_atom('_NET_WM_ICON'), X.AnyPropertyType)
            assert icon is not None and icon.format == 32, 'native application icon is missing'
            pixels = list(icon.value)
            assert pixels[:2] == [128, 128] and len(pixels) == 2 + 128 * 128
            icon_path = Path(__file__).resolve().parent.parent / 'assets/branding/window-icon.png'
            with Image.open(icon_path) as image:
                rgba = image.convert('RGBA').tobytes()
            assert pixels[2:] == [(a << 24) | (r << 16) | (g << 8) | b
                                  for r, g, b, a in zip(rgba[0::4], rgba[1::4], rgba[2::4], rgba[3::4])], \
                'window manager icon differs from the Tide artwork'
            results['gui_native_window_uses_tide_icon'] = 'passed'
            if args.scroll_selection_only:
                check_scrollback_selection()
                return
            if args.focus_only:
                check_terminal_focus()
                return
            if args.titles_only:
                check_tab_titles()
                return
            if args.close_only:
                check_close_protection()
                check_window_shutdown()
                return
            if args.chrome_only:
                check_chrome()
                return
            if args.append_only:
                check_appending()
                return
            if args.btop_only:
                check_btop()
                return
            if args.icons_only:
                check_icons()
                return
            if args.pager_only:
                check_pager()
                return
            if not args.run_input_only:
                check_scrollback_selection()
                check_close_protection()
                check_chrome()
                check_terminal_focus()
                check_tab_titles()
                check_appending()
                check_pager()
            initial_scale = ui_state()["scale"]
            click("terminal", .15, .2)
            key("r", ("Control_L", "Shift_L"))
            wait(lambda: "command" in ui_state().get("ui", {}), "Run shortcut opens dialog")
            wait(lambda: "run_shell" in ui_state().get("ui", {}), "Run identifies the selected shell")
            screenshot("run-dialog.png")
            key("a", ("Control_L",))
            key("BackSpace")
            text("sleep 30")
            field("timeout", "invalid duration")
            click("start_run")
            wait(lambda: "run_error" in ui_state().get("ui", {}), "Run validation stays inside the form")
            time.sleep(.3)
            screenshot("run-validation.png")
            click("timeout_unit")
            click("timeout_m")
            click("timeout_unit")
            click("timeout_s")
            field("timeout", "0.7")
            screenshot("run-corrected.png")
            index = len(records())
            click("start_run")
            started = wait(lambda: event_after(index, lambda e: e["event"] == "started" and e["managed"]), "keyboard-opened Run dialog accepts input")
            assert started["event"]["command"] == "sleep 30", started
            wait(lambda: event_after(index, lambda e: e["event"] == "rule_triggered" and "Runtime" in e["reason"]), "keyboard-opened Run dialog applies timer")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited"), "keyboard-opened command exits")
            results["gui_run_shortcut_typing_and_timer"] = "passed"
            results["gui_run_validation_keeps_fields_editable"] = "passed"
            results["gui_seconds_unit_selector_and_numeric_duration"] = "passed"
            if args.run_input_only:
                return
            key("w", ("Control_L", "Shift_L"))
            time.sleep(1)
            click("terminal", .15, .2)
            index = len(records())
            text("printf 'GUI_KEYBOARD_OK\\n'\n")
            wait(lambda: event_after(index, lambda e: e["event"] == "command_ended" and e["code"] == 0), "keyboard command completes")
            assert event_after(index, lambda e: e["event"] == "command_started" and "GUI_KEYBOARD_OK" in e["command"])
            results["keyboard_to_real_shell"] = "passed"

            index = len(records())
            text('python3 -u -c "import time; print(\'GUI_CONTROL_READY\',flush=True); time.sleep(30)"\n')
            # Preexec metadata can precede tcsetpgrp/exec. Wait for the child
            # itself so an interrupt cannot accidentally target the idle shell.
            wait(lambda: "GUI_CONTROL_READY\r\n" in output_since(index), "foreground child ready")
            key("c", ("Control_L",))
            wait(lambda: event_after(index, lambda e: e["event"] == "command_ended" and e["code"] == 130), "Ctrl+C interrupts the foreground command", timeout=4)
            results["gui_ctrl_c_foreground_interrupt"] = "passed"

            click("history")
            field("history_query", "GUI_KEYBOARD_OK")
            key("a", ("Control_L",))
            key("c", ("Control_L",))
            screenshot("history.png")
            # SQLite is independent evidence that the command crossed the shell adapter.
            import sqlite3
            connection = sqlite3.connect(scratch / "data/tide-terminal/history.sqlite3")
            assert connection.execute("SELECT count(*) FROM history WHERE command LIKE '%GUI_KEYBOARD_OK%'").fetchone()[0] > 0
            results["history_lookup"] = "passed"
            index=len(records())
            click("history_result_0")
            time.sleep(.3)
            assert not event_after(index,lambda e:e["event"]=="command_started"), "Selecting history executed a command"
            click("terminal",.15,.2)
            key("Return")
            wait(lambda:event_after(index,lambda e:e["event"]=="command_ended" and e["code"]==0),"selected history executes only on Enter")
            results["history_insert_without_execution"]="passed"

            time.sleep(.2)
            text("printf 'GUI_KEY")
            time.sleep(.4)
            screenshot("inline-suggestion.png")
            # Record real rendered frames around a large cursor jump.
            state=ui_state();rect=state["ui"]["terminal"];scale=state["scale"]
            crop=(round(rect[0]*scale),round(rect[1]*scale),round(rect[2]*scale),round(rect[3]*scale))
            frames=[frame().crop(crop)]
            key("Home")
            # Shell redraw and Xwayland presentation are asynchronous. A 225 ms
            # window can end on the first intermediate frame, before the target
            # is presented. Sample the complete transition; retain both assertions.
            deadline = time.monotonic() + 1.2
            while time.monotonic() < deadline:
                frames.append(frame().crop(crop));time.sleep(.025)
            frames[0].save(args.output_dir / "cursor-motion.gif",save_all=True,append_images=frames[1:],duration=40,loop=0)
            def cursor_x(image):
                red, green, blue = image.convert('RGB').split()
                threshold = red.point(lambda value: min(255, int(value * 1.3)))
                nonzero = lambda value: 255 if value > 0 else 0
                mask = ImageChops.multiply(ImageChops.subtract(green, threshold).point(nonzero),
                                          ImageChops.subtract(blue, threshold).point(nonzero))
                mask = ImageChops.multiply(mask, green.point(lambda value: 255 if value > 80 else 0))
                bounds = mask.getbbox()
                if not bounds:
                    return None
                cropped = mask.crop(bounds)
                xs = [bounds[0] + index % cropped.width for index, value in enumerate(cropped.tobytes()) if value]
                return sum(xs)/len(xs) if xs else None
            positions=[x for x in map(cursor_x,frames) if x is not None]
            assert len(positions)>=3 and max(positions)-min(positions)>10,positions
            assert any(min(positions)+1<x<max(positions)-1 for x in positions),positions
            results["rendered_smooth_cursor_motion"]="passed"
            # Native editors may dismiss a suggestion after Home; request it again by editing.
            key("End");key("u",("Control_L",));text("printf 'GUI_KEY");time.sleep(.4)
            key("Right");index=len(records());key("Return")
            wait(lambda:event_after(index,lambda e:e["event"]=="command_ended" and e["code"]==0),"GUI inline suggestion accepted")
            assert event_after(index,lambda e:e["event"]=="command_started" and "GUI_KEYBOARD_OK" in e["command"])
            results["gui_inline_suggestion_acceptance"]="passed"

            index = start_run('python3 -u -c "import time; print(\'GUI_TIMER_RUNNING\'); time.sleep(30)"', timeout="700ms")
            wait(lambda: event_after(index, lambda e: e["event"] == "rule_triggered" and "Runtime" in e["reason"]), "GUI timer fires")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited"), "GUI timer stops command")
            screenshot("timer-rule.png")
            results["gui_timer_stop"] = "passed"

            index = start_run('python3 -u -c "import time; print(\'GUI_FINISHED\'); time.sleep(30)"', marker="GUI_FINISHED")
            wait(lambda: event_after(index, lambda e: e["event"] == "rule_triggered" and "marker" in e["reason"]), "GUI output rule fires")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited"), "GUI output rule stops command")
            results["gui_output_stop"] = "passed"

            index = start_run('python3 -u -c "import getpass; getpass.getpass(\'Password: \'); print(\'GUI_PASSWORD_ACCEPTED\')"')
            wait(lambda: event_after(index, lambda e: e["event"] == "attention" and "password" in e["message"]), "GUI password badge")
            # Session events precede the next rendered frame. Verify the visible
            # amber badge/banner before capturing evidence or sending a password.
            def rendered_attention():
                image = native_frame().convert("RGB")
                colors = image.crop((0, 40, image.width, min(220, image.height))).getcolors(image.width * image.height)
                return sum(count for count, (r, g, b) in colors if r > 180 and 110 < g < 220 and b < 150) > 30
            wait(rendered_attention, "rendered amber input alert")
            screenshot("input-alert.png")
            click("terminal", .15, .25)
            text("gui-secret-never-log\n")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "password input resumes program")
            assert "gui-secret-never-log" not in log.read_text()
            results["password_alert_input_and_no_echo"] = "passed"

            # Exercise the actual GUI path, not just the byte encoder. Raw mode
            # exposes every byte, including controls normally handled by the tty.
            import shlex
            chords = [(chr(letter), ("Control_L",), letter - 96) for letter in range(97, 123)]
            chords += [("bracketleft", ("Control_L",), 27), ("backslash", ("Control_L",), 28),
                       ("bracketright", ("Control_L",), 29), ("6", ("Control_L",), 30),
                       ("minus", ("Control_L", "Shift_L"), 31), ("slash", ("Control_L", "Shift_L"), 127),
                       ("space", ("Control_L",), 0), ("2", ("Control_L", "Shift_L"), 0),
                       ("8", ("Control_L",), 127), ("BackSpace", ("Control_L",), 8)]
            expected = bytes(byte for _, _, byte in chords) + b"GUI_KEYBOARD_OK"
            raw_script = f"import os,termios,tty; old=termios.tcgetattr(0); tty.setraw(0); print('RAW_KEYS_READY',flush=True); data=b''.join(os.read(0,1) for _ in range({len(expected)})); termios.tcsetattr(0,termios.TCSANOW,old); print('RAW_KEYS_'+data.hex(),flush=True)"
            index = start_run("python3 -u -c " + shlex.quote(raw_script))
            wait(lambda: event_after(index, lambda e: e["event"] == "output" and "RAW_KEYS_READY" in e["text"]), "raw key receiver ready")
            click("terminal", .15, .25)
            for name, modifiers, _ in chords:
                key(name, modifiers)
            key("v", ("Control_L", "Shift_L"))
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "all control bytes received")
            output = "".join(r["event"]["text"] for r in records()[index:] if r.get("event", {}).get("event") == "output")
            assert "RAW_KEYS_" + expected.hex() in output, output
            results["gui_all_legacy_control_bytes"] = "passed"
            results["gui_ctrl_shift_clipboard_paste"] = "passed"
            assert abs(ui_state()["scale"] - initial_scale) < .001, "Control punctuation changed UI zoom"
            results["gui_control_keys_preserve_ui_scale"] = "passed"

            index = start_run('python3 -u -c "import time; print(\'Place your finger on the fingerprint reader\',flush=True); time.sleep(3); print(\'GUI_SCAN_COMPLETE\',flush=True); time.sleep(.7)"')
            wait(lambda: event_after(index, lambda e: e["event"] == "attention" and "fingerprint" in e["message"]), "newline-terminated fingerprint prompt")
            wait(rendered_attention, "rendered fingerprint alert")
            screenshot("fingerprint-alert.png")
            scanned_index = len(records())
            # No key is sent: real fingerprint completion comes from the reader.
            wait(lambda: event_after(scanned_index, lambda e: e["event"] == "output" and "GUI_SCAN_COMPLETE" in e["text"]), "external scan completion")
            wait(lambda: event_after(scanned_index, lambda e: e["event"] == "attention_cleared"), "fingerprint attention clears on progress")
            wait(lambda: event_after(index, lambda e: e["event"] == "exited" and e["code"] == 0), "fingerprint fixture completes")
            results["gui_fingerprint_alert_and_external_completion"] = "passed"

            if args.btop_binary:
                check_btop()
            if args.eza_binary:
                check_icons()

            click("settings")
            # Click the slider track, rather than its text/value editor.
            click("opacity", .15)
            click("save_settings")
            import tomllib
            config_path = scratch / ".tide.toml"
            config = tomllib.loads(config_path.read_text())
            assert config_path.stat().st_mode & 0o777 == 0o600
            assert config["gpu"] == args.gpu, config
            assert config["font_file"] == "", config
            results["dotfile_preferences_and_private_permissions"] = "passed"
            font_path = Path(__file__).resolve().parent.parent / "assets/fonts/DejaVuSans.ttf"
            click("font_file")
            text(str(font_path))
            click("save_settings")
            assert tomllib.loads(config_path.read_text())["font_file"] == str(font_path)
            results["font_file_preference_saved_for_restart"] = "passed"
            screenshot("appearance.png")
            assert .3 <= config["opacity"] < .9, config
            results["live_opacity_and_saved_settings"] = "passed"
            key("Escape")
            canvas=args.output_dir / "canvas.png"
            old=canvas.stat().st_mtime_ns if canvas.exists() else 0
            key("s",("Control_L","Shift_L"))
            def completed_canvas():
                if not canvas.exists() or canvas.stat().st_mtime_ns==old:return None
                try:
                    with Image.open(canvas) as image:
                        image.load()
                        return image.convert("RGBA")
                except (OSError,ValueError):return None
            rgba=wait(completed_canvas,"GPU screenshot after opacity change")
            assert 70<rgba.getpixel((rgba.width//2,rgba.height-120))[3]<240
            results["gpu_background_alpha"]="passed"

            index = len(records())
            key("t", ("Control_L", "Shift_L"))
            wait(lambda: event_after(index, lambda e: e["event"] == "started" and not e["managed"]), "new shell shortcut")
            results["new_tab_shortcut"] = "passed"

            def header_fits():
                state = ui_state()
                geometry = window.get_geometry()
                for name in ["new_tab", "run", "history", "settings", "tab_strip"]:
                    if name not in state.get("ui", {}):
                        return False
                    x1, y1, x2, y2 = state["ui"][name]
                    if not (0 <= x1 < x2 <= geometry.width / state["scale"] and 0 <= y1 < y2 <= 48):
                        return False
                return True
            wait(header_fits, "single-row tab overflow keeps actions visible")
            results["gui_single_row_tab_overflow"] = "passed"

            window.configure(width=640, height=420)
            d.sync()
            wait(lambda: window.get_geometry().width == 640 and window.get_geometry().height == 420, "minimum window size")
            wait(header_fits, "compact header controls remain reachable")
            def active_tab_visible():
                snapshot = ui_state()
                state = snapshot.get("ui", {})
                current = 'tab_' + str(snapshot.get('active_tab'))
                if current not in state or "tab_strip" not in state:
                    return False
                tab, strip = state[current], state["tab_strip"]
                return strip[0] - 1 <= tab[0] and tab[2] <= strip[2] + 1
            wait(active_tab_visible, "resizing keeps the active tab fully visible")
            click("settings")
            def compact_dialog_fits(control="save_settings"):
                state = ui_state()
                if control not in state.get("ui", {}):
                    return False
                x1, y1, x2, y2 = state["ui"][control]
                return 0 <= x1 < x2 <= 640 / state["scale"] and 0 <= y1 < y2 <= 420 / state["scale"]
            wait(compact_dialog_fits, "settings actions fit the minimum window")
            screenshot("compact-settings.png")
            key("Escape")
            click("run")
            wait(lambda: compact_dialog_fits("start_run"), "run action fits the minimum window")
            screenshot("compact-run.png")
            key("Escape")
            click("history")
            wait(lambda: compact_dialog_fits("history_query"), "history search fits the minimum window")
            screenshot("compact-history.png")
            key("Escape")
            window.configure(width=1120, height=740)
            d.sync()
            wait(lambda: window.get_geometry().width == 1120 and window.get_geometry().height == 740, "restore full window size")
            window.set_input_focus(X.RevertToParent, X.CurrentTime)
            d.sync()
            wait(header_fits, "full header after restoring the window")
            results["gui_minimum_window_layout"] = "passed"

            # A clean preview of the actual application and real project files.
            # Close only this test's finished tabs; keep its original shell.
            while len(ui_state()['tabs']) > 1:
                close_fixture_tab()
            click("settings")
            click("opacity", .98)
            key("Escape")
            index = len(records())
            app_root = args.binary.resolve().parent.parent
            if app_root.name == "target":
                app_root = Path(__file__).resolve().parent.parent
            text("cd " + shlex.quote(str(app_root)) + "; printf '\\033[2J\\033[H'; pwd; printf '\\n'; ls --color=always\n")
            wait(lambda: event_after(index, lambda e: e["event"] == "command_ended" and e["code"] == 0), "preview shell command completes")
            time.sleep(.45)
            screenshot("terminal.png")
            check_window_shutdown()
        finally:
            try:
                if process.poll() is None:
                    # Request a normal window close so session cleanup runs.
                    from Xlib.protocol import event
                    close = event.ClientMessage(window=window, client_type=d.intern_atom("WM_PROTOCOLS"),
                                                data=(32, [d.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]))
                    window.send_event(close, event_mask=0)
                    d.flush()
                    try:
                        deadline = time.monotonic() + 3
                        while process.poll() is None and time.monotonic() < deadline:
                            if 'close_confirm' in ui_state().get('ui', {}):
                                click('close_confirm')
                                break
                            time.sleep(.05)
                        process.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        process.terminate()
                        process.wait(timeout=5)
                    except AssertionError:
                        if process.poll() is None:
                            raise
            finally:
                if log.exists():
                    (args.output_dir / "events.jsonl").write_text(log.read_text())
                (args.output_dir / "gui-results.json").write_text(json.dumps(results, indent=2) + "\n")
                if helper:
                    helper.stdin.close();helper.wait(timeout=3)
        print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
