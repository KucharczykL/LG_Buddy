#!/usr/bin/env python3
"""Observe installed GTK plus the config owner's first screen-daemon start."""

import argparse
import hashlib
import importlib
import json
import os
import signal
import subprocess
import time
from pathlib import Path

import pyatspi

ui = importlib.import_module('test-release-gui-accessibility')


def snapshot(path):
    facts = path.stat()
    return path.read_bytes(), facts.st_uid, facts.st_gid, facts.st_mode & 0o7777


def stop(process):
    if process is not None and process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--isolated-prefix', type=Path, required=True)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--token', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    prefix = args.isolated_prefix.resolve()
    if os.geteuid() == 0 or prefix == Path('/') or not os.environ.get('DISPLAY') or not os.environ.get('DBUS_SESSION_BUS_ADDRESS'):
        parser.error('requires a normal user, isolated installation prefix, display and private session bus')
    runtime, gui = prefix / 'usr/bin/lg-buddy', prefix / 'usr/bin/lg-buddy-gui'
    before, credential = snapshot(args.config), snapshot(args.token)
    assert before[1] == os.geteuid() and credential[1] == os.geteuid()
    assert b'tvs_primary_platform=bscpylgtv' in before[0] and b'screen_backend=swayidle' in before[0]
    expected = before[0].replace(b'tvs_primary_platform=bscpylgtv', b'tvs_primary_platform=lg_webos').replace(b'screen_backend=swayidle', b'screen_backend=auto')
    args.evidence.mkdir()
    environment = {**os.environ, 'LG_BUDDY_CONFIG': str(args.config), 'GTK_A11Y': 'atspi'}
    environment.pop('NO_AT_BRIDGE', None)
    monitor = application = None
    record = {'outcome': 'failed', 'uid': os.geteuid(),
              'boundary': 'isolated prefix and private session/display; real installed GUI and screen daemon; no TV required'}
    try:
        with (args.evidence / 'gui.output').open('wb') as output:
            application = subprocess.Popen([str(gui)], env=environment, stdout=output, stderr=subprocess.STDOUT,
                                           start_new_session=True)
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                window = subprocess.run(['xdotool', 'search', '--onlyvisible', '--pid', str(application.pid), '--name', '^LG Buddy$'],
                                        capture_output=True, timeout=3)
                if window.returncode == 0:
                    break
                assert application.poll() is None, 'installed GUI exited before opening'
                time.sleep(.1)
            else:
                raise AssertionError('installed GUI did not open')
            assert snapshot(args.config) == before and snapshot(args.token) == credential
            record['gui_open_preserved_stale_config'] = True
            # This is the ordinary per-user service entrypoint. The GUI remains
            # open while the owner starts it; no foreground migration is invoked.
            with (args.evidence / 'screen.output').open('wb') as output:
                monitor = subprocess.Popen([str(runtime), 'monitor'], env=environment, stdout=output,
                                           stderr=subprocess.STDOUT, start_new_session=True)
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline and args.config.read_bytes() != expected:
                    assert monitor.poll() is None, 'screen daemon exited before conversion'
                    time.sleep(.1)
                assert snapshot(args.config) == (expected, *before[1:]), 'startup did not perform only the supported conversion'
                assert snapshot(args.token) == credential, 'startup changed native credential bytes or metadata'
                deadline = time.monotonic() + 15
                labels = set()
                while time.monotonic() < deadline:
                    apps = [app for app in pyatspi.Registry.getDesktop(0)
                            if ui.name(app) in {'lg-buddy-gui', '.lg-buddy-gui-wrapped'}]
                    labels = {ui.name(item) for app in apps for item in ui.accessible_tree(app) if ui.is_showing(item)}
                    if labels:
                        break
                    time.sleep(.1)
                assert labels, 'installed GUI had no observable accessibility tree'
                assert not any('migrat' in label.lower() for label in labels), labels
                assert application.poll() is None, 'installed GUI exited during startup conversion'
                record.update(outcome='passed', gui_pid=application.pid, screen_pid=monitor.pid,
                              config_access=before[1:], credential_access=credential[1:],
                              config_before_sha256=hashlib.sha256(before[0]).hexdigest(),
                              config_after_sha256=hashlib.sha256(expected).hexdigest(),
                              credential_sha256=hashlib.sha256(credential[0]).hexdigest(),
                              gui_controls=sorted(labels))
    finally:
        stop(monitor)
        stop(application)
        (args.evidence / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print('Installed GUI/startup migration passed: payload and GUI preserve stale data; owner daemon converts locally without migration acknowledgement.')


if __name__ == '__main__':
    main()
