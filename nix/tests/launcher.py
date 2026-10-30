"""Launch cached standalone Home Manager entries through real GIO."""
import os
from pathlib import Path
import sys

import gi

gi.require_version("GioUnix", "2.0")
from gi.repository import GioUnix, GLib


first_exe, second_exe = sys.argv[-2:]
log = Path.home() / "launches"
expected = []


def launch(app):
    pids = []
    app.launch_uris_as_manager([], None, GLib.SpawnFlags.DO_NOT_REAP_CHILD,
                               None, None, lambda _app, pid, _data: pids.append(pid), None)
    assert len(pids) == 1, pids
    return os.waitstatus_to_exitcode(os.waitpid(pids[0], 0)[1])


for offset in range(1, len(sys.argv) - 2, 3):
    profile, first, second = map(Path, sys.argv[offset:offset + 3])
    profile.parent.mkdir(parents=True, exist_ok=True)
    profile.symlink_to(first)
    desktop = profile / "share/applications/io.github.staphylococcus.LGBuddy.desktop"
    cached = GioUnix.DesktopAppInfo.new_from_filename(str(desktop))
    assert cached is not None, desktop
    for generation, executable in [(first, first_exe), (second, second_exe), (first, first_exe)]:
        pending = profile.with_name("next-profile")
        pending.symlink_to(generation)
        os.replace(pending, profile)
        assert launch(cached) == 0
        expected.append(executable)
        observed = log.read_text().splitlines()
        assert observed == expected, (observed, expected)
    profile.unlink()
    assert launch(cached) == 127
    assert log.read_text().splitlines() == expected
print("Cached home launchers followed upgrade and rollback; removal refused the stale launch.")
