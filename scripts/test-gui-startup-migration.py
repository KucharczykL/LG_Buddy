#!/usr/bin/env python3
"""Prove installed GUI startup converts stale config without a daemon or TV."""

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
    gui = prefix / 'usr/bin/lg-buddy-gui'
    before, credential = snapshot(args.config), snapshot(args.token)
    assert before[1] == os.geteuid() and credential[1] == os.geteuid()
    assert b'tvs_primary_platform=bscpylgtv' in before[0] and b'screen_backend=swayidle' in before[0]
    expected = before[0].replace(b'tvs_primary_platform=bscpylgtv', b'tvs_primary_platform=lg_webos').replace(b'screen_backend=swayidle', b'screen_backend=auto')
    args.evidence.mkdir()
    environment = {**os.environ, 'LG_BUDDY_CONFIG': str(args.config), 'GTK_A11Y': 'atspi'}
    environment.pop('NO_AT_BRIDGE', None)
    application = None
    record = {'outcome': 'failed', 'uid': os.geteuid(),
              'boundary': 'isolated prefix and private session/display; real installed GUI; no daemon or TV; service activation remains unqualified',
              'launches': []}
    try:
        probe_mtime = args.config.stat().st_mtime_ns
        for probe in ('--version', '--check-runtime'):
            subprocess.run([str(gui), probe], env=environment, check=True, capture_output=True, timeout=10)
            assert snapshot(args.config) == before and args.config.stat().st_mtime_ns == probe_mtime
            assert snapshot(args.token) == credential
        # Repeated current startup must not rewrite; restored legacy contents
        # must convert through every ordinary GUI entry, including handoff.
        for name, arguments, restore in (
            ('overview', [], True), ('current', [], False),
            ('brightness', ['brightness'], True), ('relaunch', ['--gapplication-replace'], True),
        ):
            if restore:
                args.config.write_bytes(before[0])
            current_mtime = args.config.stat().st_mtime_ns
            with (args.evidence / f'{name}.output').open('wb') as output:
                application = subprocess.Popen([str(gui), *arguments], env=environment, stdout=output,
                                               stderr=subprocess.STDOUT, start_new_session=True)
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline and args.config.read_bytes() != expected:
                    assert application.poll() is None, 'installed GUI exited before conversion'
                    time.sleep(.1)
                assert snapshot(args.config) == (expected, *before[1:]), 'GUI startup did not perform only the supported conversion'
                assert snapshot(args.token) == credential, 'GUI startup changed native credential bytes or metadata'
                if not restore:
                    assert args.config.stat().st_mtime_ns == current_mtime, 'current GUI startup rewrote config'
                deadline = time.monotonic() + 15
                labels = set()
                while time.monotonic() < deadline:
                    assert application.poll() is None, 'installed GUI exited before presenting its normal state'
                    apps = [app for app in pyatspi.Registry.getDesktop(0)
                            if ui.name(app) in {'lg-buddy-gui', '.lg-buddy-gui-wrapped'}]
                    labels = {ui.name(item) for app in apps for item in ui.accessible_tree(app) if ui.is_showing(item)}
                    # This fixture deliberately has no running service. Require
                    # the same ordinary setup state for converted and current
                    # input, rather than mistaking any accessible window for
                    # readiness or fabricating a successful setup assessment.
                    if {'Setup required', 'Complete setup'} <= labels:
                        break
                    time.sleep(.1)
                else:
                    raise AssertionError(f'GUI did not present the expected unactivated-service state: {labels}')
                assert not any('migrat' in label.lower() for label in labels), labels
                assert application.poll() is None, 'installed GUI exited during startup conversion'
                record['launches'].append({'entry': name, 'gui_pid': application.pid,
                                          'state': 'setup-required (services not activated)', 'controls': sorted(labels)})
            stop(application)
            application = None
        record.update(outcome='passed', config_access=before[1:], credential_access=credential[1:],
                      config_before_sha256=hashlib.sha256(before[0]).hexdigest(),
                      config_after_sha256=hashlib.sha256(expected).hexdigest(),
                      credential_sha256=hashlib.sha256(credential[0]).hexdigest())
    finally:
        stop(application)
        (args.evidence / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print('Installed GUI startup migration passed: overview, brightness and update handoff convert locally without a daemon; current startup and probes preserve data.')


if __name__ == '__main__':
    main()
