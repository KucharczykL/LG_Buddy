"""Keep one real desktop launch object alive across Nix generation switches."""
import sys

import dbus.mainloop.glib
import dbus.service
import gi

gi.require_version("GioUnix", "2.0")
from gi.repository import GioUnix, GLib


desktop, name = sys.argv[1:]
# Deliberately never reload the desktop file, even after a generation change.
app = GioUnix.DesktopAppInfo.new_from_filename(desktop)
assert app is not None, desktop
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)


class Launcher(dbus.service.Object):
    @dbus.service.method("io.github.staphylococcus.CachedLauncher", in_signature="", out_signature="")
    def Launch(self):
        assert app.launch([], None)


bus = dbus.SessionBus()
owner = dbus.service.BusName(name, bus)
launcher = Launcher(bus, "/launcher")
GLib.MainLoop().run()
