#!/usr/bin/env python3
"""Package input, layout and version-ordering contracts (no root required)."""

from __future__ import annotations

import io
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from build_debian_package import (
    GUI_TARGET, ICON, POLICY, SYSTEM_UNITS, USER_UNITS,
    PackageError, debian_version, stage_payload,
)
from release_bundle_manifest import ManifestError, ReleaseIdentity, render_manifest
from release_promotion import SemVer
from test_release_bundle_manifest import embedded_binary


REPO = Path(__file__).resolve().parent.parent
IDENTITY = ReleaseIdentity(
    release_tag="v2.0.0-beta.2", version="2.0.0-beta.2", channel="prerelease",
    target="x86_64-unknown-linux-musl", gui_target=GUI_TARGET, commit="a" * 40,
)


def fixture_members(identity: ReleaseIdentity = IDENTITY) -> dict[str, bytes]:
    members = {
        "release-manifest.json": render_manifest(identity),
        "lg-buddy": embedded_binary(identity, identity.target),
        f"docs/lg-buddy-gui-{GUI_TARGET}": embedded_binary(identity, GUI_TARGET),
        "bin/LG_Buddy_Common": (REPO / "bin/LG_Buddy_Common").read_bytes(),
        "docs/setup-services.sh": (REPO / "data/setup-services.sh").read_bytes(),
        f"docs/{POLICY}": (REPO / f"data/{POLICY}").read_bytes(),
        f"docs/{ICON}": (REPO / f"data/icons/hicolor/scalable/apps/{ICON}").read_bytes(),
        "LG_Buddy_Brightness.desktop": (REPO / "io.github.staphylococcus.LGBuddy.desktop").read_bytes(),
        "README.md": b"# LG Buddy\n", "LICENSE": (REPO / "LICENSE").read_bytes(),
        "docs/package-ownership.md": (REPO / "docs/package-ownership.md").read_bytes(),
    }
    for unit in (*SYSTEM_UNITS, *USER_UNITS, "lg_buddy.conf"):
        members[f"systemd/{unit}"] = (REPO / "systemd" / unit).read_bytes()
    for directory in ("kwin", "gnome-shell"):
        for path in sorted((REPO / "data" / directory).rglob("*")):
            if path.is_file():
                members["docs/" + str(path.relative_to(REPO / "data"))] = path.read_bytes()
    members["docs/kwin/prebuilt/example/plugin.so"] = b"optional prebuilt input"
    return members


def archive_fixture(directory: Path, members: dict[str, bytes], *, extra: tarfile.TarInfo | None = None) -> Path:
    prefix = f"lg-buddy-{IDENTITY.version}-{IDENTITY.target}"
    archive = directory / f"{prefix}.tar.gz"
    with tarfile.open(archive, "w:gz") as bundle:
        for name, content in members.items():
            entry = tarfile.TarInfo(f"{prefix}/{name}")
            entry.size = len(content)
            entry.mode = 0o755
            bundle.addfile(entry, io.BytesIO(content))
        if extra is not None:
            bundle.addfile(extra, io.BytesIO(b"x" * extra.size))
    return archive


class VersionTests(unittest.TestCase):
    def test_native_versions_and_packaging_revisions(self) -> None:
        self.assertEqual(debian_version("2.0.0"), "2.0.0-1")
        self.assertEqual(debian_version("2.0.0-beta.2.ci", 2), "2.0.0~beta.2.ci-2")

    @unittest.skipUnless(shutil.which("dpkg"), "native version ordering requires dpkg")
    def test_prerelease_and_revision_ordering_with_dpkg(self) -> None:
        versions = ["2.0.0-alpha.2", "2.0.0-alpha.10", "2.0.0-beta.1", "2.0.0-beta.1.ci", "2.0.0-rc.1", "2.0.0", "2.0.1"]
        for older, newer in zip(versions, versions[1:]):
            subprocess.run(["dpkg", "--compare-versions", debian_version(older), "lt", debian_version(newer)], check=True)
        subprocess.run(["dpkg", "--compare-versions", debian_version("2.0.0"), "lt", debian_version("2.0.0", 2)], check=True)

    def test_ambiguous_ordering_or_metadata_is_refused(self) -> None:
        for version in (
            "2.0.0-alpha10", "2.0.0-alpha-1", "2.0.0+build.1",
            "2.0.0-alpha.a.1", "2.0.0-alpha.aa", "2.0.0-beta.1.2",
            "2.0.0-preview.1",
        ):
            with self.subTest(version=version), self.assertRaises(PackageError):
                debian_version(version)
        with self.assertRaises(PackageError):
            debian_version("2.0.0", 0)

    @unittest.skipUnless(shutil.which("dpkg"), "native version ordering requires dpkg")
    def test_all_supported_prerelease_forms_keep_native_order(self) -> None:
        versions = []
        for channel in ("alpha", "beta", "rc"):
            versions.append(f"2.0.0-{channel}")
            for number in (0, 1, 2, 9, 10, 100):
                versions.extend((f"2.0.0-{channel}.{number}", f"2.0.0-{channel}.{number}.ci"))
            versions.append(f"2.0.0-{channel}.ci")
        versions.append("2.0.0")
        self.assertEqual(versions, sorted(versions, key=SemVer.parse))
        for older, newer in zip(versions, versions[1:]):
            with self.subTest(older=older, newer=newer):
                subprocess.run(["dpkg", "--compare-versions", debian_version(older), "lt", debian_version(newer)], check=True)


class PayloadTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.root = self.directory / "package"

    def stage(self, members: dict[str, bytes], *, extra: tarfile.TarInfo | None = None) -> ReleaseIdentity:
        return stage_payload(archive_fixture(self.directory, members, extra=extra), self.root)

    def test_complete_layout_preserves_verifiable_vendor_assets(self) -> None:
        self.assertEqual(self.stage(fixture_members()), IDENTITY)
        for scope, units in (("system", SYSTEM_UNITS), ("user", USER_UNITS)):
            for unit in units:
                base = self.root / f"usr/lib/systemd/{scope}/{unit}"
                template = self.root / f"usr/lib/lg-buddy/setup/systemd/{unit}"
                self.assertEqual(base.read_bytes(), (REPO / "systemd" / unit).read_bytes())
                self.assertEqual(template.read_bytes(), base.read_bytes())
        hook = self.root / "usr/lib/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle"
        self.assertEqual(hook.read_text(), '#!/bin/sh\nset -eu\n[ "${2:-}" = pre-down ] || exit 0\nexec /usr/bin/lg-buddy nm-pre-down\n')
        self.assertTrue((self.root / "usr/lib/lg-buddy/kwin/prebuilt/example/plugin.so").is_file())
        self.assertTrue((self.root / "usr/lib/lg-buddy/gnome-shell/lg-buddy-brightness@staphylococcus.github.io/extension.js").is_file())
        self.assertEqual((self.root / "usr/share/doc/lg-buddy/copyright").read_bytes(), (REPO / "LICENSE").read_bytes())
        for name in ("etc", "home", "var", "run"):
            self.assertFalse((self.root / name).exists(), name)
        self.assertFalse((self.root / "usr/lib/lg-buddy/config-path").exists())
        for path in self.root.rglob("*"):
            if path.is_file():
                expected = 0o755 if path.name in ("lg-buddy", "lg-buddy-gui", "setup-services", "setup.sh", "build.sh", "common.sh", "LG_Buddy_lifecycle") else 0o644
                self.assertEqual(path.stat().st_mode & 0o777, expected, str(path))

    def test_required_asset_missing_or_directory_refuses_packaging(self) -> None:
        for name in ("systemd/LG_Buddy_screen.service", "docs/setup-services.sh", "docs/kwin/source/main.cpp"):
            with self.subTest(name=name):
                members = fixture_members()
                del members[name]
                with self.assertRaisesRegex(PackageError, "required package input missing"):
                    self.stage(members)
                entry = tarfile.TarInfo(f"lg-buddy-{IDENTITY.version}-{IDENTITY.target}/{name}")
                entry.type = tarfile.DIRTYPE
                with self.assertRaisesRegex(PackageError, "required package input missing"):
                    self.stage(members, extra=entry)

    def test_unsafe_extra_members_never_escape_staging(self) -> None:
        prefix = f"lg-buddy-{IDENTITY.version}-{IDENTITY.target}"
        for name in ("/tmp/escape", f"{prefix}/docs/../../escape", f"{prefix}/docs//escape", "other/escape"):
            with self.subTest(name=name):
                entry = tarfile.TarInfo(name)
                entry.size = 1
                with self.assertRaises(PackageError):
                    self.stage(fixture_members(), extra=entry)
        self.assertFalse((self.directory / "escape").exists())

    def test_links_and_duplicate_payloads_are_refused(self) -> None:
        prefix = f"lg-buddy-{IDENTITY.version}-{IDENTITY.target}"
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
            entry = tarfile.TarInfo(f"{prefix}/docs/redirect")
            entry.type = kind
            entry.linkname = "/etc/passwd"
            with self.subTest(kind=kind), self.assertRaises(PackageError):
                self.stage(fixture_members(), extra=entry)
        entry = tarfile.TarInfo(f"{prefix}/docs/setup-services.sh")
        entry.size = 1
        with self.assertRaises(PackageError):
            self.stage(fixture_members(), extra=entry)

    def test_mismatched_binary_identity_is_refused(self) -> None:
        members = fixture_members()
        members[f"docs/lg-buddy-gui-{GUI_TARGET}"] = embedded_binary(replace(IDENTITY, commit="b" * 40), GUI_TARGET)
        with self.assertRaises(ManifestError):
            self.stage(members)


if __name__ == "__main__":
    unittest.main()
