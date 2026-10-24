#!/usr/bin/env python3
"""Check installed native-package startup migration without desktop or TV access."""

import os, pathlib, pwd, signal, stat, subprocess, sys, time

assert os.geteuid() == 0 and pathlib.Path("/.dockerenv").exists() and os.environ.get("LG_BUDDY_DISPOSABLE_PACKAGE_TEST") == "1", "requires an explicitly disposable root Docker environment"
config, log = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[3])
user = pwd.getpwnam(sys.argv[2])
token = config.parent / "tvs/primary/access-token.json"
runtime = "/usr/bin/lg-buddy"

def snapshot(path):
    metadata = path.stat()
    return (path.read_bytes(), metadata.st_uid, metadata.st_gid,
            stat.S_IMODE(metadata.st_mode))

before, credential = snapshot(config), snapshot(token)
assert user.pw_uid != 0 and before[1:] == (user.pw_uid, user.pw_gid, 0o640)
environment = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "HOME": "/root",
               "LG_BUDDY_CONFIG": str(config)}
as_owner = ["/usr/sbin/runuser", "-u", user.pw_name, "--", "/usr/bin/env", "-i",
            "PATH=/usr/sbin:/usr/bin:/sbin:/bin", f"HOME={user.pw_dir}",
            f"LG_BUDDY_CONFIG={config}"]
# Lifecycle opens its bus before loading config; an empty private bus reaches
# the real stale-config guard without supplying logind or a system manager.
root_guard = ["/usr/bin/dbus-run-session", "--", "/bin/sh", "-c",
              'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"; '
              'exec /usr/bin/lg-buddy lifecycle']
for command in (root_guard, [*as_owner, runtime, "volume"]):
    refused = subprocess.run(command, env=environment, capture_output=True, timeout=15)
    assert refused.returncode != 0 and b"v2 migration required" in refused.stderr, refused
    assert snapshot(config) == before and snapshot(token) == credential

expected = before[0].replace(b"tvs_primary_platform=bscpylgtv",
                             b"tvs_primary_platform=lg_webos").replace(
                                 b"screen_backend=swayidle", b"screen_backend=auto")
with log.open("wb") as output:
    monitor = subprocess.Popen([*as_owner, runtime, "monitor"], env=environment,
                               stdout=output, stderr=subprocess.STDOUT,
                               start_new_session=True)
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if config.read_bytes() == expected or monitor.poll() is not None:
                break
            time.sleep(0.1)
    finally:
        if monitor.poll() is None:
            try:
                os.killpg(monitor.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                monitor.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(monitor.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                monitor.wait(timeout=3)

assert snapshot(config) == (expected, *before[1:]), log.read_text(errors="replace")
assert snapshot(token) == credential
print("Installed startup migration passed: root/foreground refusal, non-root conversion, retained preferences and credentials.")
