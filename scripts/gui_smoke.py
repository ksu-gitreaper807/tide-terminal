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

from PIL import Image
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
    parser.add_argument("--append-only", action="store_true")
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
        process = subprocess.Popen([str(args.binary), "gui", "--gpu", args.gpu, "--qa", "--clean", "--shell", "/bin/bash",
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
            field("timeout", timeout)
            field("idle", "")
            field("marker", marker)
            field("pattern", "")
            index = len(records())
            click("start_run")
            wait(lambda: event_after(index, lambda e: e["event"] == "started" and e["managed"]), "managed process started")
            return index

        def check_btop():
            import shlex
            # Keep process command lines out of diagnostic logs and screenshots.
            config_dir = scratch / "config/btop"
            config_dir.mkdir(parents=True, exist_ok=True)
            (config_dir / "btop.conf").write_text('shown_boxes = "cpu mem"\ngraph_symbol = "braille"\nupdate_ms = 500\n')
            help_text = subprocess.run([str(args.btop_binary), "--help"], env=env, capture_output=True, text=True, timeout=5, check=True).stdout
            utf_flag = next((flag for flag in ("--force-utf", "--utf-force") if flag in help_text), None)
            assert utf_flag, "btop --help advertises no supported force-UTF option"
            index = start_run(shlex.quote(str(args.btop_binary)) + " " + utf_flag, shortcut=args.btop_only)
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
            time.sleep(.4);assert not forbidden.exists()
            assert not event_after(appended,lambda e:e["event"]=="started")
            results["gui_append_source_close_cancels_queue"]="passed"
            def tab_count():
                return sum(name.startswith("tab_") and name[4:].isdigit() for name in ui_state().get("ui",{}))
            while tab_count()>1:
                previous=tab_count();key("w",("Control_L","Shift_L"))
                wait(lambda:tab_count()<previous,"close append test tabs")

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

        try:
            wait(lambda: event_after(0, lambda e: e["event"] == "prompt"), "initial shell prompt")
            gpu=wait(lambda:next((r['gpu'] for r in records() if 'gpu' in r),None),'actual GPU adapter')
            expected = {"integrated": "IntegratedGpu", "discrete": "DiscreteGpu", "software": "Cpu"}.get(args.gpu)
            if expected:
                assert gpu['device_type'] == expected, gpu
            results['integrated_gpu_rendering' if args.gpu == 'integrated' else 'explicit_renderer'] = gpu
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
            for _ in range(9):
                frames.append(frame().crop(crop));time.sleep(.025)
            frames[0].save(args.output_dir / "cursor-motion.gif",save_all=True,append_images=frames[1:],duration=40,loop=0)
            def cursor_x(image):
                xs=[x for y in range(image.height) for x in range(image.width) if (lambda c:c[1]>c[0]*1.3 and c[2]>c[0]*1.3 and c[1]>80)(image.getpixel((x,y)))]
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
                    if not (0 <= x1 < x2 <= geometry.width / state["scale"] and 0 <= y1 < y2 <= 45):
                        return False
                return True
            wait(header_fits, "single-row tab overflow keeps actions visible")
            results["gui_single_row_tab_overflow"] = "passed"

            window.configure(width=640, height=420)
            d.sync()
            wait(lambda: window.get_geometry().width == 640 and window.get_geometry().height == 420, "minimum window size")
            wait(header_fits, "compact header controls remain reachable")
            def active_tab_visible():
                state = ui_state().get("ui", {})
                if "tab_6" not in state or "tab_strip" not in state:
                    return False
                tab, strip = state["tab_6"], state["tab_strip"]
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
            def tab_count():
                return sum(name.startswith("tab_") and name[4:].isdigit() for name in ui_state().get("ui", {}))
            while tab_count() > 1:
                previous = tab_count()
                key("w", ("Control_L", "Shift_L"))
                wait(lambda: tab_count() < previous, "test tab closes before the next shortcut")
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
        finally:
            # Request a normal window close so session cleanup runs.
            from Xlib.protocol import event
            close = event.ClientMessage(window=window, client_type=d.intern_atom("WM_PROTOCOLS"),
                                        data=(32, [d.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]))
            window.send_event(close, event_mask=0)
            d.flush()
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)
            if log.exists():
                (args.output_dir / "events.jsonl").write_text(log.read_text())
            (args.output_dir / "gui-results.json").write_text(json.dumps(results, indent=2) + "\n")
            if helper:
                helper.stdin.close();helper.wait(timeout=3)
        print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
