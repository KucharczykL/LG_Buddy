#!/usr/bin/env python3
"""Exercise the installed updater through APT or DNF in a disposable container.

The baseline uses the existing debug-only transport fixtures at version 0.0.0.
Only GitHub HTTP transport and administrator authentication are substituted.
Candidate acquisition, root receipt, APT, installed verification and GUI handoff
use the application paths. Real polkit/session/reboot qualification is separate.
"""

from __future__ import annotations

import argparse
import json
import os
import pty
import pwd
import select
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
from mock_github_gui import Fixture, Handler, ThreadingHTTPServer, archive_facts  # noqa: E402


def checked(*command: str, **kwargs: object) -> str:
    result = subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=120, **kwargs)
    return result.stdout


def drive(answer: str, version: str) -> str:
    assert os.geteuid() != 0, "GUI handoff must stay in the invoking user's session"
    master, slave = pty.openpty()
    process = subprocess.Popen(["/usr/bin/lg-buddy", "updates", "install"],
                               stdin=slave, stdout=slave, stderr=slave)
    os.close(slave)
    output = bytearray()
    sent = False
    deadline = time.monotonic() + 120
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    block = os.read(master, 8192)
                except OSError:
                    break
                if not block:
                    break
                output.extend(block)
                assert len(output) < 128 * 1024, "unexpectedly large updater output"
                if not sent and b"Type `yes`" in output:
                    os.write(master, (answer + "\n").encode())
                    sent = True
            elif process.poll() is not None:
                break
        assert sent, output.decode(errors="replace")
        assert process.wait(timeout=5) == 0, output.decode(errors="replace")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
    text = output.decode(errors="replace")
    if answer != "yes":
        assert "Upgrade cancelled." in text, text
        return text
    assert f"Installed: {version} " in text, text
    for _ in range(200):
        window = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^LG Buddy$"],
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if window.returncode == 0:
            break
        time.sleep(0.1)
    else:
        raise AssertionError("the verified installed GUI was not relaunched")
    pids = checked("pgrep", "-u", str(os.getuid()), "-x", "lg-buddy-gui").splitlines()
    assert len(pids) == 1, pids
    assert os.readlink(f"/proc/{pids[0]}/exe") == "/usr/bin/lg-buddy-gui"
    checked("gapplication", "action", "io.github.staphylococcus.LGBuddy", "quit")
    return text


def snapshot(paths: list[Path]) -> list[tuple[bytes, int, int, int]]:
    return [(path.read_bytes(), path.stat().st_uid, path.stat().st_gid,
             path.stat().st_mode & 0o7777) for path in paths]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--format", choices=("debian", "rpm"), default="debian")
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--runtime-fixture", type=Path)
    parser.add_argument("--gui-fixture", type=Path)
    parser.add_argument("--expect-new-rpm-dependency", help="require this repository dependency to be absent before, and installed after, the update")
    parser.add_argument("--drive", choices=("yes", "no"))
    parser.add_argument("--version", default="")
    parser.add_argument("--drive-output", type=Path)
    args = parser.parse_args()
    if args.drive:
        text = drive(args.drive, args.version)
        if args.drive_output:
            args.drive_output.write_text(text)
        return
    assert os.geteuid() == 0 and Path("/.dockerenv").exists() and os.environ.get("LG_BUDDY_DISPOSABLE_PACKAGE_TEST") == "1", "requires an explicitly disposable root Docker environment"
    os.umask(0o022)
    assert all((args.archive, args.package, args.runtime_fixture, args.gui_fixture))
    rpm = args.format == "rpm"
    manager = "DNF" if rpm else "APT"
    protocol = "apply-rpm-update" if rpm else "apply-debian-update"
    def installed_version() -> str:
        return checked("rpm", "--query", "--queryformat", "%{VERSION}-%{RELEASE}", "lg-buddy") if rpm else checked("dpkg-query", "-W", "-f=${Version}", "lg-buddy")
    facts = archive_facts(args.archive)
    username = "lgdeb-update"
    checked("useradd", "--create-home", "--shell", "/bin/bash", username)
    user = pwd.getpwnam(username)
    sudoers = Path("/etc/sudoers.d/lg-buddy-debian-update-test")
    sudoers.write_text(f'Defaults:{username} env_keep += "LG_BUDDY_TEST_GITHUB_ADDRESS"\n{username} ALL=(root) NOPASSWD: /usr/bin/lg-buddy {protocol} *\n')
    sudoers.chmod(0o440)
    checked("visudo", "--check", "--file", str(sudoers))
    try:
        with tempfile.TemporaryDirectory(prefix="lg-buddy-debian-update-") as directory:
            work = Path(directory)
            root = work / "baseline"
            (root / "DEBIAN").mkdir(parents=True)
            (root / "usr/bin").mkdir(parents=True)
            (root / "DEBIAN/control").write_text("Package: lg-buddy\nVersion: 0.0.0-1\nArchitecture: amd64\nMaintainer: LG Buddy <7261268+Staphylococcus@users.noreply.github.com>\nDescription: debug transport baseline for native updater smoke\n")
            for name, source in (("lg-buddy", args.runtime_fixture), ("lg-buddy-gui", args.gui_fixture)):
                destination = root / "usr/bin" / name
                shutil.copyfile(source, destination)
                destination.chmod(0o755)
                if rpm:
                    checked("strip", "--strip-debug", str(destination))
                assert checked(str(destination), "--version").startswith("lg-buddy 0.0.0\n")
            if rpm:
                spec = work / "baseline.spec"
                spec.write_text(f"""Name: lg-buddy
Version: 0.0.0
Release: 1
Epoch: 0
Summary: Native updater debug transport baseline
License: GPL-3.0-only
BuildArch: x86_64
%global debug_package %{{nil}}
%global __os_install_post %{{nil}}
%global _build_id_links none
%description
Only the transport fixture baseline for a disposable native updater smoke.
%install
mkdir -p %{{buildroot}}/usr/bin
cp {root}/usr/bin/* %{{buildroot}}/usr/bin/
%files
%defattr(-,root,root,-)
/usr/bin/lg-buddy
/usr/bin/lg-buddy-gui
""")
                checked("rpmbuild", "--define", f"_topdir {work}/rpm-build", "--define", "_rpmformat 6", "--define", "_smp_build_ncpus 1", "--define", "_binary_payload w9.gzdio", "-bb", str(spec))
                baseline = work / "rpm-build/RPMS/x86_64/lg-buddy-0.0.0-1.x86_64.rpm"
                checked("dnf5", "--assumeyes", "--setopt=tsflags=", "install", str(baseline))
            else:
                baseline = work / "lg-buddy_0.0.0-1_amd64.deb"
                checked("dpkg-deb", "--root-owner-group", "-Zgzip", "--build", str(root), str(baseline))
                checked("apt-get", "install", "-y", "--no-install-recommends", str(baseline), env={**os.environ, "DEBIAN_FRONTEND": "noninteractive"})
            if args.expect_new_rpm_dependency:
                assert rpm
                absent = subprocess.run(["rpm", "--query", args.expect_new_rpm_dependency], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                assert absent.returncode == 1, "expected a genuinely new repository dependency"
            checked("runuser", "-u", username, "--", "/usr/bin/lg-buddy", "settings", "set", "updates.channel", "prerelease")
            config = Path(user.pw_dir) / ".config/lg-buddy/config.env"
            token = config.parent / "tvs/primary/access-token.json"
            token.parent.mkdir(parents=True)
            token.write_text('{"access_token":"retained-native-update-test-token"}\n')
            token.chmod(0o600)
            for path in (token.parent.parent, token.parent, token):
                os.chown(path, user.pw_uid, user.pw_gid)
            binding = Path("/etc/lg-buddy/config-path")
            binding.parent.mkdir(parents=True, exist_ok=True)
            binding.write_text(str(config) + "\n")
            retained = [config, token, binding]
            before = snapshot(retained)
            state = work / "state.json"
            state.write_text('{"mode":"available"}\n')
            requests = work / "requests.jsonl"
            fixture = Fixture(args.archive, state, requests, "0.0.0", args.package)
            server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            server.fixture = fixture
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            environment = {**os.environ, "LG_BUDDY_TEST_GITHUB_ADDRESS": f"127.0.0.1:{server.server_address[1]}"}
            try:
                # Supplying bytes is insufficient authority: the root helper
                # must independently verify the published digest before APT.
                corrupt = work / "corrupt.deb"
                data = bytearray(args.package.read_bytes())
                data[-1] ^= 1
                corrupt.write_bytes(data)
                with corrupt.open("rb") as stream:
                    refused = subprocess.run(["/usr/bin/lg-buddy", protocol, str(facts["tag"]), str(facts["commit"])],
                                             stdin=stream, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                             env=environment, timeout=60)
                assert refused.returncode != 0 and b"mutation_started" not in refused.stdout, (refused.stdout, refused.stderr)
                assert requests.exists() and b"digest" in refused.stderr, (refused.stdout, refused.stderr)
                assert installed_version() == "0.0.0-1"
                assert snapshot(retained) == before
                if rpm:
                    checked("dnf5", "versionlock", "add", "lg-buddy")
                    with args.package.open("rb") as stream:
                        held = subprocess.run(["/usr/bin/lg-buddy", protocol, str(facts["tag"]), str(facts["commit"])], stdin=stream, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment, timeout=120)
                    assert held.returncode != 0 and b"mutation_started" not in held.stdout and b"version locks" in held.stderr, (held.stdout, held.stderr)
                    assert installed_version() == "0.0.0-1" and snapshot(retained) == before
                    checked("dnf5", "versionlock", "delete", "lg-buddy")
                prior_requests = len(requests.read_text().splitlines())
                for answer in ("no", "yes"):
                    driver_log = Path(user.pw_dir) / f"update-{answer}.output"
                    checked("runuser", "-u", username, "--", "dbus-run-session", "--", "xvfb-run", "-a",
                            "python3", str(Path(__file__).resolve()), "--drive", answer, "--version", str(facts["version"]),
                            "--drive-output", str(driver_log), env=environment)
                    expected_output = "Upgrade cancelled." if answer == "no" else f'Installed: {facts["version"]} '
                    assert expected_output in driver_log.read_text()
                    assert snapshot(retained) == before
                    if answer == "no":
                        recent = requests.read_text().splitlines()[prior_requests:]
                        assert not any("/releases/assets/" in line for line in recent), recent
                expected_version = str(facts["version"]).replace("-", "~", 1) + "-1"
                assert installed_version() == expected_version
                if args.expect_new_rpm_dependency:
                    print(checked("rpm", "--query", args.expect_new_rpm_dependency), end="", flush=True)
                    assert not checked("rpm", "--verify", args.expect_new_rpm_dependency)
                for binary in ("lg-buddy", "lg-buddy-gui"):
                    text = checked(f"/usr/bin/{binary}", "--version")
                    assert f'version: {facts["version"]}\n' in text and f'commit: {facts["commit"]}\n' in text
                    if rpm:
                        assert checked("rpm", "--query", "--file", "--queryformat", "%{NAME}", f"/usr/bin/{binary}") == "lg-buddy"
                    else:
                        assert checked("dpkg-query", "-S", f"/usr/bin/{binary}").strip() == f"lg-buddy: /usr/bin/{binary}"
                paths = [json.loads(line)["path"] for line in requests.read_text().splitlines()]
                assert paths.count("/api.github.com/repos/Staphylococcus/LG_Buddy/releases/assets/1200") == 1
                assert paths.count("/api.github.com/repos/Staphylococcus/LG_Buddy/releases/assets/1201") >= 3
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
    finally:
        sudoers.unlink(missing_ok=True)
    print(f"Native {manager} updater passed: root digest refusal, decline, verified package upgrade, retained data and unprivileged GTK relaunch.")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(f"{error.cmd}: {error.stdout}\n{error.stderr}")
