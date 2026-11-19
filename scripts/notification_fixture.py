#!/usr/bin/env python3
"""Private notification server for GUI regression tests; never uses the desktop bus."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time


class Fixture:
    def __init__(self, scratch, backend):
        self.log = scratch / 'notification-events.jsonl'
        self.bus = subprocess.Popen(['dbus-daemon', '--session', '--nofork', '--print-address=1'], stdout=subprocess.PIPE, text=True)
        self.address = self.bus.stdout.readline().strip()
        env = os.environ.copy()
        env['DBUS_SESSION_BUS_ADDRESS'] = self.address
        env['GDK_BACKEND'] = backend
        self.process = subprocess.Popen(['/usr/bin/python3', __file__, '--serve', str(self.log)], env=env)
        deadline = time.monotonic() + 10
        while not self.records():
            if self.process.poll() is not None or time.monotonic() > deadline:
                self.close()
                raise AssertionError('Private notification fixture did not start')
            time.sleep(.05)

    def records(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def arm(self, identifier):
        subprocess.run(['gdbus', 'call', '--address', self.address, '--dest', 'org.freedesktop.Notifications',
                        '--object-path', '/org/freedesktop/Notifications', '--method', 'org.tide.Test.Arm', str(identifier)], check=True, stdout=subprocess.DEVNULL)

    def close(self):
        for process in (getattr(self, 'process', None), self.bus):
            if process is not None and process.poll() is None:
                process.terminate()
                process.wait(timeout=5)


def serve(path):
    import gi
    gi.require_version('Gtk', '3.0')
    from gi.repository import Gio, GLib, Gtk, GdkPixbuf
    path = Path(path)
    def log(record):
        with path.open('a') as file:
            file.write(json.dumps(record) + '\n')
    xml = '''<node><interface name="org.freedesktop.Notifications">
    <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
    <method name="CloseNotification"><arg type="u" direction="in"/></method>
    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
    <signal name="ActivationToken"><arg type="u"/><arg type="s"/></signal>
    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
    </interface><interface name="org.tide.Test"><method name="Arm"><arg type="u" direction="in"/></method></interface></node>'''
    connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    reply = connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', ('org.freedesktop.Notifications', 4)), GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 3000, None)
    assert reply.unpack() == (1,), 'Refusing to replace an existing notification service'
    window = Gtk.Window(title='Tide notification QA')
    window.set_default_size(480, 200)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
    box.set_border_width(20)
    image = Gtk.Image()
    label = Gtk.Label(label='Input needed')
    button = Gtk.Button(label='Open Tide')
    box.pack_start(image, False, False, 0)
    box.pack_start(label, True, True, 0)
    box.pack_start(button, False, False, 0)
    window.add(box)
    pending = {}
    current = [0]
    next_id = [0]
    def clicked(_):
        identifier = current[0]
        context = window.get_display().get_app_launch_context()
        context.set_timestamp(Gtk.get_current_event_time())
        info = Gio.AppInfo.create_from_commandline('tide-terminal', 'Tide', Gio.AppInfoCreateFlags.SUPPORTS_STARTUP_NOTIFICATION)
        token = context.get_startup_notify_id(info, [])
        if token:
            connection.emit_signal(None, '/org/freedesktop/Notifications', 'org.freedesktop.Notifications', 'ActivationToken', GLib.Variant('(us)', (identifier, token)))
        connection.emit_signal(None, '/org/freedesktop/Notifications', 'org.freedesktop.Notifications', 'ActionInvoked', GLib.Variant('(us)', (identifier, 'default')))
        connection.flush_sync(None)
        log({'clicked': identifier, 'activation_token_received': bool(token)})
    button.connect('clicked', clicked)
    def method(conn, sender, object_path, interface, name, params, invocation):
        args = params.unpack()
        if name == 'Notify':
            app, replaces, icon, title, body, actions, hints, timeout = args
            next_id[0] += 1
            identifier = replaces or next_id[0]
            pending[identifier] = args
            raw = hints.get('image-data')
            log({'notice': identifier, 'app': app, 'icon': icon, 'title': title, 'body': body, 'actions': actions,
                 'desktop_entry': hints.get('desktop-entry'), 'image': list(raw[:6]) if raw else None,
                 'image_sha256': __import__('hashlib').sha256(bytes(raw[6])).hexdigest() if raw else None})
            invocation.return_value(GLib.Variant('(u)', (identifier,)))
        elif name == 'CloseNotification':
            pending.pop(args[0], None)
            log({'closed': args[0]})
            invocation.return_value(None)
        elif name == 'Arm':
            current[0] = args[0]
            notice = pending[current[0]]
            label.set_text(notice[4])
            raw = notice[6].get('image-data')
            if raw:
                w, h, stride, alpha, bits, channels, rgba = raw
                pixbuf = GdkPixbuf.Pixbuf.new_from_bytes(GLib.Bytes.new(bytes(rgba)), GdkPixbuf.Colorspace.RGB, alpha, bits, w, h, stride)
                image.set_from_pixbuf(pixbuf.scale_simple(48, 48, GdkPixbuf.InterpType.BILINEAR))
            window.show_all()
            window.present()
            invocation.return_value(None)
    node = Gio.DBusNodeInfo.new_for_xml(xml)
    for interface in node.interfaces:
        connection.register_object('/org/freedesktop/Notifications', interface, method, None, None)
    log({'ready': True})
    Gtk.main()


if __name__ == '__main__':
    assert sys.argv[1] == '--serve'
    serve(sys.argv[2])
