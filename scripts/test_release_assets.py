#!/usr/bin/env python3
"""Release inventory/trust tests with contract mocks for native read-only tools."""

import base64
import io
import json
import os
import tarfile
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from release_assets import AssetError, asset_names, digest, verify_assets
from release_bundle_manifest import ManifestError, ReleaseIdentity, render_manifest, gui_bundle_path
from test_release_bundle_manifest import embedded_binary


# These mocks implement only the decoder commands consumed by release_assets.
# Real DEB/RPM decoding is additionally exercised in the native CI lanes.
DECODER = '''#!/usr/bin/env python3
import base64, json, sys
from pathlib import Path
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
if tool == 'dpkg-deb':
    package = json.loads(Path(args[1]).read_text())
    if args[0] == '--field':
        assert args[2:] == ['Package', 'Version', 'Architecture']
        sys.stdout.write(package['metadata'])
    else:
        assert args[0] in ('--fsys-tarfile', '--ctrl-tarfile') and len(args) == 2
        key = 'control' if args[0] == '--ctrl-tarfile' else 'payload'
        sys.stdout.buffer.write(base64.b64decode(package[key]))
else:
    package = json.load(sys.stdin)
    if tool == 'rpm':
        assert args[:3] == ['--query', '--package', '--queryformat'] and args[-1] == '-'
        sys.stdout.write(package['inventory'] if args[3].startswith('[') else package['metadata'])
    else:
        assert tool == 'rpm2archive' and args == ['-n', '-']
        sys.stdout.buffer.write(base64.b64decode(package['payload']))
'''


def package_fixtures(directory: Path, identity: ReleaseIdentity) -> list[Path]:
    output = io.BytesIO()
    files = {
        'usr/bin/lg-buddy': (embedded_binary(identity, identity.target), 0o755),
        'usr/bin/lg-buddy-gui': (embedded_binary(identity, identity.gui_target), 0o755),
        'usr/share/doc/lg-buddy/release-manifest.json': (render_manifest(identity), 0o644),
    }
    with tarfile.open(fileobj=output, mode='w') as payload:
        for name, (content, mode) in files.items():
            member = tarfile.TarInfo('./' + name)
            member.size, member.mode = len(content), mode
            payload.addfile(member, io.BytesIO(content))
    native = identity.version.replace('-', '~', 1) + '-1'
    control = io.BytesIO()
    with tarfile.open(fileobj=control, mode='w') as archive:
        archive.addfile(tarfile.TarInfo('control'), io.BytesIO())
    common = {'payload': base64.b64encode(output.getvalue()).decode(),
              'control': base64.b64encode(control.getvalue()).decode(),
              'inventory': ''.join(f'/{name}\t0\n' for name in files)}
    names = asset_names(identity.version)
    values = (
        {**common, 'metadata': f'Package: lg-buddy\nVersion: {native}\nArchitecture: amd64\n'},
        {**common, 'metadata': f'lg-buddy\n0\n{native}\nx86_64\n'},
    )
    paths = [directory / name for name in names[1:3]]
    for path, value in zip(paths, values):
        path.write_text(json.dumps(value))
    return paths


def decoder_fixtures(directory: Path) -> str:
    directory.mkdir()
    for name in ('dpkg-deb', 'rpm', 'rpm2archive'):
        path = directory / name
        path.write_text(DECODER)
        path.chmod(0o755)
    return f"{directory}:{os.environ['PATH']}"


class AssetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.dist = self.root / 'dist'
        self.dist.mkdir()
        self.identity = ReleaseIdentity('v2.0.0-beta.2', '2.0.0-beta.2', 'prerelease',
                                        'x86_64-unknown-linux-musl', 'a' * 40, 'x86_64-unknown-linux-gnu')
        self.names = asset_names(self.identity.version)
        self.archive = self.dist / self.names[0]
        with tarfile.open(self.archive, mode='w:gz') as bundle:
            for name, content in (
                ('release-manifest.json', render_manifest(self.identity)),
                ('lg-buddy', embedded_binary(self.identity, self.identity.target)),
                (gui_bundle_path(self.identity.gui_target), embedded_binary(self.identity, self.identity.gui_target)),
            ):
                member = tarfile.TarInfo(self.archive.name.removesuffix('.tar.gz') + '/' + name)
                member.mode, member.size = (0o644 if name == 'release-manifest.json' else 0o755), len(content)
                bundle.addfile(member, io.BytesIO(content))
        self.packages = package_fixtures(self.dist, self.identity)
        self.environment = patch.dict(os.environ, {'PATH': decoder_fixtures(self.root / 'bin')})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.checksums()

    def checksums(self):
        (self.dist / 'sha256sums.txt').write_text(''.join(
            f'{digest(self.dist / name)}  {name}\n' for name in self.names[:3]))

    def test_complete_exact_asset_set(self):
        self.assertEqual([path.name for path in verify_assets(self.dist, self.identity)], list(self.names))

    def test_generation_includes_both_native_packages(self):
        (self.dist / 'sha256sums.txt').unlink()
        verify_assets(self.dist, self.identity, write_checksums=True)
        self.assertEqual(len((self.dist / 'sha256sums.txt').read_text().splitlines()), 3)

    def test_expanded_bundle_staging_directory_is_not_a_release_asset(self):
        staging = self.dist / self.archive.name.removesuffix('.tar.gz')
        staging.mkdir()
        (staging / 'README.md').write_text('bundle staging input')
        self.assertEqual(len(verify_assets(self.dist, self.identity)), 4)

    def test_missing_and_unexpected_packages_are_refused(self):
        for name in self.names[1:3]:
            path = self.dist / name
            content = path.read_bytes()
            path.unlink()
            with self.assertRaisesRegex(AssetError, 'inventory differs'):
                verify_assets(self.dist, self.identity)
            path.write_bytes(content)
        (self.dist / 'unexpected.rpm').touch()
        with self.assertRaisesRegex(AssetError, 'unexpected'):
            verify_assets(self.dist, self.identity)

    def test_checksums_cannot_omit_duplicate_or_redirect_packages(self):
        path = self.dist / 'sha256sums.txt'
        original = path.read_text()
        lines = original.splitlines(True)
        for text in (''.join(lines[:1]), original + lines[1], original.replace(self.names[1], '../outside.deb')):
            path.write_text(text)
            with self.assertRaises(AssetError):
                verify_assets(self.dist, self.identity)
        path.write_text(original.replace(lines[1][:64], '0' * 64))
        with self.assertRaisesRegex(AssetError, 'checksum mismatch'):
            verify_assets(self.dist, self.identity)

    def test_metadata_mismatch_is_refused_even_with_matching_checksums(self):
        for path in self.packages:
            original = path.read_text()
            metadata = json.loads(original)['metadata']
            for wrong in (
                metadata.replace('lg-buddy', 'other-package'),
                metadata.replace('0\n2.0.0', '1\n2.0.0').replace('Version: 2.0.0', 'Version: 1.0.0'),
                metadata.replace('amd64', 'arm64').replace('x86_64', 'aarch64'),
                metadata.replace('~beta.2-1', '~beta.2-2'),
            ):
                value = json.loads(original)
                value['metadata'] = wrong
                path.write_text(json.dumps(value))
                self.checksums()
                with self.assertRaisesRegex(AssetError, 'disagrees'):
                    verify_assets(self.dist, self.identity)
            path.write_text(original)
        self.checksums()

    def test_wrong_embedded_commit_is_refused(self):
        package_fixtures(self.dist, replace(self.identity, commit='b' * 40))
        self.checksums()
        with self.assertRaisesRegex(ManifestError, 'embedded identity'):
            verify_assets(self.dist, self.identity)

    def test_debian_control_refuses_conffiles_links_duplicate_and_nested_entries(self):
        path = self.packages[0]
        original = json.loads(path.read_text())
        for name, kind, content, duplicate in (
            ('conffiles', tarfile.REGTYPE, b'/usr/share/doc/lg-buddy/README.md\n', False),
            ('control', tarfile.SYMTYPE, b'', False),
            ('nested/control', tarfile.REGTYPE, b'', False),
            ('../control', tarfile.REGTYPE, b'', False),
            ('nested\\control', tarfile.REGTYPE, b'', False),
            ('control', tarfile.REGTYPE, b'', True),
        ):
            with self.subTest(name=name, kind=kind, duplicate=duplicate):
                output = io.BytesIO()
                with tarfile.open(fileobj=output, mode='w') as archive:
                    for _ in range(2 if duplicate else 1):
                        member = tarfile.TarInfo(name)
                        member.type, member.size = kind, len(content)
                        archive.addfile(member, io.BytesIO(content))
                path.write_text(json.dumps({**original, 'control': base64.b64encode(output.getvalue()).decode()}))
                self.checksums()
                with self.assertRaisesRegex(AssetError, 'Debian control'):
                    verify_assets(self.dist, self.identity)

    def test_rpm_mutable_inventory_and_missing_payload_are_refused(self):
        path = self.packages[1]
        original = json.loads(path.read_text())
        for inventory in ('/etc/lg-buddy/config-path\t64\n', original['inventory'] + '/usr/ghost\t64\n',
                          original['inventory'].replace('/usr/bin/lg-buddy\t0\n', '')):
            path.write_text(json.dumps({**original, 'inventory': inventory}))
            self.checksums()
            with self.assertRaises(AssetError):
                verify_assets(self.dist, self.identity)

    def test_symlink_assets_are_refused(self):
        path = self.packages[1]
        destination = self.root / 'outside.rpm'
        path.rename(destination)
        path.symlink_to(destination)
        with self.assertRaisesRegex(AssetError, 'regular release asset'):
            verify_assets(self.dist, self.identity)

    def test_checksum_refusal_precedes_archive_and_native_decoding(self):
        self.packages[1].write_bytes(b'tampered package')
        with patch('release_assets.validate_archive') as archive, patch('release_assets.verify_package') as native:
            with self.assertRaisesRegex(AssetError, 'checksum mismatch'):
                verify_assets(self.dist, self.identity)
            archive.assert_not_called()
            native.assert_not_called()


if __name__ == '__main__':
    unittest.main()
