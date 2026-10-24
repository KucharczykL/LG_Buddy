#!/usr/bin/env python3
"""Eligibility and public-input refusal contracts for native release canaries."""

import importlib
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from release_assets import AssetError, asset_names, digest
from test_build_debian_package import IDENTITY, archive_fixture, fixture_members
from test_release_assets import decoder_fixtures, package_fixtures

canary = importlib.import_module('test-production-package-canary')


def release(version, *, native=True, draft=False):
    names = asset_names(version) if native else (f'lg-buddy-{version}-x86_64-unknown-linux-musl.tar.gz', 'sha256sums.txt')
    return {'tag_name': 'v' + version, 'draft': draft, 'prerelease': '-' in version,
            'immutable': True,
            'assets': [{'name': name, 'state': 'uploaded', 'size': 100,
                        'digest': 'sha256:' + 'a' * 64} for name in names]}


class CanaryTests(unittest.TestCase):
    def test_history_supports_old_gh_and_preserves_multiline_release_records(self):
        releases = [release('2.0.0-beta.2'), release('2.0.0-beta.1', draft=True)]
        releases[0]['body'] = 'First line\nQuoted "release" notes'
        def old_gh(*arguments):
            if '--slurp' in arguments:
                raise RuntimeError('unknown flag: --slurp')
            return '\n'.join(json.dumps(item) for item in releases)
        with patch.object(canary, 'checked', side_effect=old_gh):
            self.assertEqual(canary.release_history(), releases)

    def test_installed_verification_refuses_successful_commands_with_discrepancies(self):
        for family in ('debian', 'rpm'):
            for report in ('missing /usr/share/doc/lg-buddy/release-manifest.json',
                           '??5?????? /usr/bin/lg-buddy'):
                with self.subTest(family=family, report=report), patch.object(
                    canary, 'checked', side_effect=['2.0.0~beta.1-1', report]
                ), self.assertRaisesRegex(AssetError, 'discrepancies'):
                    canary.verify_installed_package(family)

    def test_installed_verification_accepts_clean_native_payload(self):
        for family in ('debian', 'rpm'):
            with self.subTest(family=family), patch.object(
                canary, 'checked', side_effect=['2.0.0~beta.1-1', '']
            ):
                self.assertEqual(canary.verify_installed_package(family), '2.0.0~beta.1-1')

    def public_fixture(self, root):
        remote = root / 'remote'
        remote.mkdir()
        archive_fixture(remote, fixture_members())
        package_fixtures(remote, IDENTITY)
        names = asset_names(IDENTITY.version)
        (remote / names[-1]).write_text(''.join(f'{digest(remote / name)}  {name}\n' for name in names[:3]))
        candidate = release(IDENTITY.version)
        for asset in candidate['assets']:
            path = remote / asset['name']
            asset.update(size=path.stat().st_size, digest='sha256:' + digest(path))
        def download(*arguments):
            self.assertEqual(arguments[:4], ('gh', 'release', 'download', IDENTITY.release_tag))
            destination = Path(arguments[arguments.index('--dir') + 1])
            for index, argument in enumerate(arguments):
                if argument == '--pattern':
                    name = arguments[index + 1]
                    shutil.copyfile(remote / name, destination / name)
        return candidate, download

    def test_verified_public_download_matches_both_native_formats(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            candidate, download = self.public_fixture(root)
            with patch.dict(os.environ, {'PATH': decoder_fixtures(root / 'tools')}), patch.object(canary, 'api', return_value={'sha': IDENTITY.commit}), patch.object(canary, 'checked', side_effect=download):
                for family in ('debian', 'rpm'):
                    identity, package = canary.download_release(candidate, root / family, family)
                    self.assertEqual(identity, IDENTITY)
                    self.assertEqual(package.suffix, '.deb' if family == 'debian' else '.rpm')
                    self.assertEqual(len(list((root / family).iterdir())), 3)

    def test_corrupted_public_download_refuses_before_native_decoding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            candidate, download = self.public_fixture(root)
            def corrupt(*arguments):
                download(*arguments)
                destination = Path(arguments[arguments.index('--dir') + 1])
                path = destination / asset_names(IDENTITY.version)[1]
                content = bytearray(path.read_bytes())
                content[0] ^= 1
                path.write_bytes(content)
            with patch.object(canary, 'api', return_value={'sha': IDENTITY.commit}), patch.object(canary, 'checked', side_effect=corrupt), patch.object(canary, 'verify_package') as native:
                with self.assertRaisesRegex(AssetError, 'differs from GitHub asset digest'):
                    canary.download_release(candidate, root / 'download', 'debian')
                native.assert_not_called()

    def test_chooses_highest_older_public_prerelease(self):
        beta1, beta2 = release('2.0.0-beta.1'), release('2.0.0-beta.2')
        history = [beta1, release('2.0.0-rc.1', draft=True), beta2,
                   release('2.0.0-beta.3'), release('1.10.2')]
        self.assertEqual(canary.select_baseline(history, '2.0.0-beta.3'), beta2)

    def test_stable_target_uses_real_prerelease_baseline(self):
        baseline = release('2.0.0-rc.2')
        self.assertEqual(canary.select_baseline([release('2.0.0'), baseline], '2.0.0'), baseline)

    def test_first_native_release_is_explicit_bootstrap(self):
        self.assertIsNone(canary.select_baseline([release('2.0.0-beta.1'),
                                                  release('1.10.2', native=False)], '2.0.0-beta.1'))

    def test_old_stable_package_cannot_be_reported_as_bootstrap(self):
        with self.assertRaisesRegex(AssetError, 'no complete published prerelease'):
            canary.select_baseline([release('2.0.0')], '2.1.0-beta.1')

    def test_incomplete_older_package_release_cannot_be_bootstrap(self):
        baseline = release('2.0.0-beta.1')
        baseline['assets'].pop()
        with self.assertRaises(AssetError):
            canary.select_baseline([baseline], '2.0.0-beta.2')

    def test_public_input_refuses_draft_mutable_and_wrong_classification(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(canary, 'api', return_value={'sha': 'a' * 40}), patch.object(canary, 'checked') as commands:
            for field, value in (('draft', True), ('immutable', False), ('prerelease', False)):
                candidate = release('2.0.0-beta.1')
                candidate[field] = value
                with self.subTest(field=field), self.assertRaises(AssetError):
                    canary.download_release(candidate, Path(temporary) / 'download', 'rpm')
            commands.assert_not_called()

    def test_public_input_refuses_missing_duplicate_incomplete_or_unhashed_asset(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(canary, 'api', return_value={'sha': 'a' * 40}), patch.object(canary, 'checked') as commands:
            for mode in ('missing', 'duplicate', 'incomplete', 'digest'):
                candidate = release('2.0.0-beta.1')
                if mode == 'missing':
                    candidate['assets'].pop()
                elif mode == 'duplicate':
                    candidate['assets'].append(candidate['assets'][0])
                else:
                    candidate['assets'][1]['state' if mode == 'incomplete' else 'digest'] = 'invalid'
                with self.subTest(mode=mode), self.assertRaises(AssetError):
                    canary.download_release(candidate, Path(temporary) / 'download', 'debian')
            commands.assert_not_called()


if __name__ == '__main__':
    unittest.main()
