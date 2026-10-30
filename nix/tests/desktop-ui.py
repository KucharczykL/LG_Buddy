"""Observe the actual installed GTK application on the native VM desktop."""
import importlib
import json
import sys
import time
from pathlib import Path

import dbus
import pyatspi

recovery = importlib.import_module("test-native-setup-recovery")
ui = recovery.ui
scenario = sys.argv[1]


def sensitivity_path(item):
    path = []
    while item is not None and ui.role(item) != pyatspi.ROLE_APPLICATION:
        path.append({"name": ui.name(item), "role": ui.role_name(item),
                     "sensitive": ui.is_sensitive(item)})
        item = item.parent
    return path


if scenario == "identity":
    bus = dbus.SessionBus()
    api = dbus.Interface(bus.get_object("org.freedesktop.DBus", "/org/freedesktop/DBus"),
                         "org.freedesktop.DBus")
    pid = int(api.GetConnectionUnixProcessID("io.github.staphylococcus.LGBuddy"))
    executable = Path(f"/proc/{pid}/exe").resolve()
    expected = Path(sys.argv[2]) / "bin/.lg-buddy-gui-wrapped"
    assert executable == expected, (executable, expected)
    status = Path(f"/proc/{pid}/status").read_text()
    parent = int(next(line.split()[1] for line in status.splitlines() if line.startswith("PPid:")))
    runtime = Path(f"/proc/{parent}/exe").resolve()
    assert runtime == Path(sys.argv[2]) / "bin/lg-buddy", runtime
    print(json.dumps({"pid": pid, "executable": str(executable), "runtime": str(runtime)}))
elif scenario == "inhibit":
    bus = dbus.SessionBus()
    if bus.name_has_owner("org.gnome.SessionManager"):
        api = dbus.Interface(bus.get_object("org.gnome.SessionManager", "/org/gnome/SessionManager"),
                             "org.gnome.SessionManager")
        cookie = api.Inhibit("LG Buddy qualification", dbus.UInt32(0), "native idle inhibition", dbus.UInt32(8))
        assert api.IsInhibited(dbus.UInt32(8))
    else:
        api = dbus.Interface(bus.get_object("org.freedesktop.ScreenSaver", "/ScreenSaver"),
                             "org.freedesktop.ScreenSaver")
        cookie = api.Inhibit("LG Buddy qualification", "native idle inhibition")
        pd = dbus.Interface(bus.get_object("org.kde.Solid.PowerManagement",
                            "/org/kde/Solid/PowerManagement/PolicyAgent"),
                            "org.kde.Solid.PowerManagement.PolicyAgent")
        recovery.wait("effective PowerDevil inhibition", lambda: pd.HasInhibition(dbus.UInt32(4)), 15)
    Path("/tmp/qualification-inhibitor-ready").touch()
    while True:
        time.sleep(1)
elif scenario == "assessment":
    bus = dbus.SessionBus()
    if "recheck" in sys.argv[3:]:
        proxy = bus.get_object(recovery.BUS_NAME, recovery.SESSION_PATH, introspect=False)
        instance, revision = dbus.Interface(proxy, recovery.SESSION_INTERFACE).RequestSetupAssessment(timeout=2)
        def reassessed():
            current = recovery.snapshot(bus)
            return current if current["instance"] == instance and current["revision"] >= revision else None
        result = recovery.wait("requested setup reassessment", reassessed)
    else:
        result = recovery.snapshot(bus)
    print(json.dumps(result))
    assert result["status"] == sys.argv[2], result
elif scenario == "gate":
    recovery.wait("setup gate", lambda: "Complete setup" in recovery.names())
    assert not recovery.functional()
elif scenario == "settings":
    recovery.wait("functional GUI", recovery.functional)
    def select_settings():
        for item in recovery.controls():
            if ui.name(item) == "Settings":
                try:
                    if item.queryAction().doAction(0):
                        return True
                except NotImplementedError:
                    pass
        return False

    recovery.wait("Settings navigation action", select_settings)
    expected = sys.argv[2]
    editable = "editable" in sys.argv[3:]

    def settings_ready():
        controls = recovery.controls()
        entries = [item for item in controls if ui.is_text_control(item)
                   and ui.name(item) == "Idle timeout"]
        if not entries or ui.text_value(entries[0]) != expected:
            return False
        # AT-SPI exposes the child's own state even when its preferences row
        # disables the subtree. Observe the effective native control hierarchy.
        path = sensitivity_path(entries[0])
        assert all(node["sensitive"] for node in path) == editable, path
        names = {ui.name(item) for item in controls}
        assert "Managed externally" in names
        notice = "Some settings are disabled because they are managed declaratively in your nix configuration"
        assert (notice in names) != editable
        toggles = [item for item in controls if ui.is_toggle_control(item)
                   and ui.name(item) == "Idle blanking"]
        assert toggles and ui.is_sensitive(toggles[0])
        return [{"name": ui.name(item), "role": ui.role_name(item),
                 "sensitive": ui.is_sensitive(item), "text": ui.text_value(item)}
                for item in controls if ui.name(item)]

    print(json.dumps(recovery.wait("settings values and sensitivity", settings_ready)))
else:
    raise AssertionError(scenario)
