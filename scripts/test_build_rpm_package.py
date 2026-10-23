#!/usr/bin/env python3
"""RPM version ordering and shared payload contracts, without installation."""

from __future__ import annotations

import unittest

import test_build_debian_package as debian_tests
from build_rpm_package import LICENSE, render_spec, rpm_version
from package_payload import PackageError, stage_payload
from release_promotion import SemVer

try:
    import rpm
except ImportError:
    rpm = None


class VersionTests(unittest.TestCase):
    def test_native_versions_and_packaging_revisions(self) -> None:
        self.assertEqual(rpm_version("2.0.0"), "2.0.0-1")
        self.assertEqual(rpm_version("2.0.0-beta.2.ci", 2), "2.0.0~beta.2.ci-2")

    def test_ambiguous_ordering_and_metadata_are_refused(self) -> None:
        for version in (
            "2.0.0-alpha.ci", "2.0.0-alpha10", "2.0.0-alpha-1",
            "2.0.0+build.1", "2.0.0-alpha.a.1", "2.0.0-alpha.aa",
            "2.0.0-beta.1.2", "2.0.0-preview.1",
        ):
            with self.subTest(version=version), self.assertRaises(PackageError):
                rpm_version(version)
        with self.assertRaises(PackageError):
            rpm_version("2.0.0", 0)

    @unittest.skipIf(rpm is None, "native ordering requires python3-rpm")
    def test_all_supported_prereleases_keep_native_order(self) -> None:
        versions = []
        for channel in ("alpha", "beta", "rc"):
            versions.append(f"2.0.0-{channel}")
            for number in (0, 1, 2, 9, 10, 100):
                versions.extend((f"2.0.0-{channel}.{number}", f"2.0.0-{channel}.{number}.ci"))
        versions.extend(("2.0.0", "2.0.1"))
        self.assertEqual(versions, sorted(versions, key=SemVer.parse))
        native = [("0", *rpm_version(version).rsplit("-", 1)) for version in versions]
        native.insert(-1, ("0", "2.0.0", "2"))
        for older, newer in zip(native, native[1:]):
            with self.subTest(older=older, newer=newer):
                self.assertLess(rpm.labelCompare(older, newer), 0)


class PayloadTests(debian_tests.PayloadTests):
    def stage(self, members, *, extra=None):
        return stage_payload(
            debian_tests.archive_fixture(self.directory, members, extra=extra), self.root,
            license_path=LICENSE, version_validator=rpm_version,
        )

    def test_complete_layout_preserves_verifiable_vendor_assets(self) -> None:
        self.assertEqual(self.stage(debian_tests.fixture_members()), debian_tests.IDENTITY)
        self.assertEqual((self.root / LICENSE).read_bytes(), (debian_tests.REPO / "LICENSE").read_bytes())
        self.assertFalse((self.root / "usr/share/doc/lg-buddy/copyright").exists())
        for name in ("etc", "home", "var", "run"):
            self.assertFalse((self.root / name).exists(), name)

    def test_archive_filenames_cannot_expand_recipe_placeholders(self) -> None:
        members = debian_tests.fixture_members()
        members["docs/@POST@"] = b"ordinary documentation"
        self.stage(members)
        spec = render_spec(self.root, rpm_version(debian_tests.IDENTITY.version))
        self.assertIn("%doc /usr/share/doc/lg-buddy/docs/@POST@\n", spec)
        self.assertEqual(spec.count("\n%post\n"), 1)


if __name__ == "__main__":
    unittest.main()
