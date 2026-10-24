#!/usr/bin/env python3
"""Exercise public native-package updates in an explicitly disposable container."""

from __future__ import annotations

import argparse
import json
import os
import pwd
import re
import subprocess
from pathlib import Path

from record_github_release_responses import API_VERSION, GitHubRecorder, record_responses
from release_assets import (
    AssetError, asset_names, digest, expected_identity, read_checksums, verify_package,
)
from release_bundle_manifest import validate_archive
from release_promotion import SemVer

REPOSITORY = 'Staphylococcus/LG_Buddy'
SCRIPTS = Path(__file__).resolve().parent


def checked(*arguments: str, env=None) -> str:
    try:
        result = subprocess.run(arguments, check=True, capture_output=True, text=True,
                                timeout=180, env=env)
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f'{arguments[0]} failed: {error.stdout[-8192:]}\n{error.stderr[-8192:]}') from error
    return result.stdout.strip()


def api(path: str):
    return json.loads(checked('gh', 'api', '--header', f'X-GitHub-Api-Version: {API_VERSION}', path))


def release_history() -> list[dict]:
    # Ubuntu 24.04's gh supports pagination/jq but predates --slurp.
    records = checked('gh', 'api', '--paginate', '--jq', '.[] | @json',
                      f'repos/{REPOSITORY}/releases?per_page=100')
    return [json.loads(line) for line in records.splitlines()]


def select_baseline(releases: list[dict], version: str) -> dict | None:
    """Choose test input only; the installed app still owns update discovery."""
    candidate = SemVer.parse(version)
    eligible = []
    older_native = False
    for release in releases:
        tag = release.get('tag_name', '')
        if release.get('draft') or not tag.startswith('v'):
            continue
        try:
            parsed = SemVer.parse(tag[1:])
        except RuntimeError:
            continue
        if parsed >= candidate:
            continue
        names = [asset.get('name', '') for asset in release.get('assets', [])]
        if not any(name.endswith(('.deb', '.rpm')) for name in names):
            continue
        older_native = True
        if release.get('prerelease') and parsed.prerelease:
            wanted = set(asset_names(tag[1:]))
            if set(names) == wanted and len(names) == len(wanted):
                eligible.append((parsed, release))
    if eligible:
        return max(eligible, key=lambda item: item[0])[1]
    if older_native:
        raise AssetError('Older native releases exist but no complete published prerelease can qualify an update')
    return None


def verify_release_metadata(release: dict, expected):
    names = asset_names(expected.version)
    if (release.get('tag_name') != expected.release_tag
        or release.get('draft') is not False or release.get('immutable') is not True
        or release.get('prerelease') != (expected.channel == 'prerelease')):
        raise AssetError('Canary input must be an immutable public release with matching classification')
    assets = release['assets']
    if len(assets) != len(names) or {asset['name'] for asset in assets} != set(names):
        raise AssetError('Published release does not have the exact four-asset inventory')
    metadata = {asset['name']: asset for asset in assets}
    for name, asset in metadata.items():
        if (asset.get('state') != 'uploaded' or not 0 < asset.get('size', 0) <= 64 * 1024 * 1024
            or not re.fullmatch(r'sha256:[0-9a-f]{64}', asset.get('digest', ''))):
            raise AssetError(f'Unverified public asset metadata: {name}')
    return metadata


def download_release(release: dict, directory: Path, family: str):
    tag = release['tag_name']
    commit = api(f'repos/{REPOSITORY}/commits/{tag}')['sha']
    expected = expected_identity(tag, commit)
    names = asset_names(expected.version)
    metadata = verify_release_metadata(release, expected)
    selected = (names[0], names[1 if family == 'debian' else 2], names[3])
    directory.mkdir()
    command = ['gh', 'release', 'download', tag, '--repo', REPOSITORY, '--dir', str(directory)]
    for name in selected:
        command.extend(('--pattern', name))
    checked(*command)
    for name in selected:
        path = directory / name
        if path.stat().st_size != metadata[name]['size'] or 'sha256:' + digest(path) != metadata[name]['digest']:
            raise AssetError(f'Public download differs from GitHub asset digest: {name}')
    recorded = read_checksums(directory / names[3], names[:3])
    for name in names[:3]:
        if 'sha256:' + recorded[name] != metadata[name]['digest']:
            raise AssetError(f'Published checksum and asset digest disagree: {name}')
    if validate_archive(directory / names[0]) != expected:
        raise AssetError('Published baseline bundle identity differs from its tag')
    package = directory / selected[1]
    verify_package(package, expected)
    return expected, package


def snapshot(paths: list[Path]):
    return [(path.read_bytes(), path.stat().st_uid, path.stat().st_gid,
             path.stat().st_mode & 0o7777) for path in paths]


def verify_installed_package(family: str) -> str:
    if family == 'rpm':
        version = checked('rpm', '-q', '--qf', '%{VERSION}-%{RELEASE}', 'lg-buddy')
        report = checked('rpm', '--verify', 'lg-buddy')
    else:
        version = checked('dpkg-query', '-W', '-f=${Version}', 'lg-buddy')
        report = checked('dpkg', '--verify', 'lg-buddy')
    # dpkg reports missing/corrupt payloads on stdout even with exit status 0.
    if report:
        raise AssetError(f'Installed package verification reported discrepancies: {report}')
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--format', choices=('debian', 'rpm'), required=True)
    parser.add_argument('--expected-tag', required=True)
    parser.add_argument('--expected-commit', required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    args = parser.parse_args()
    if not (os.geteuid() == 0 and Path('/.dockerenv').exists()
            and os.environ.get('LG_BUDDY_DISPOSABLE_PACKAGE_TEST') == '1'):
        parser.error('requires an explicitly disposable root Docker environment')
    if any(name.startswith('LG_BUDDY_') and name != 'LG_BUDDY_DISPOSABLE_PACKAGE_TEST'
           for name in os.environ):
        parser.error('production canaries reject runtime overrides')
    os.umask(0o022)
    expected = expected_identity(args.expected_tag, args.expected_commit)
    work = args.work_dir.resolve()
    work.mkdir(parents=True, exist_ok=False)
    record = {'outcome': 'failed', 'format': args.format, 'target': args.expected_tag,
              'target_commit': args.expected_commit,
              'substitutions': 'disposable container, passwordless sudo and Xvfb; real public GitHub and package managers'}
    sudoers = Path('/etc/sudoers.d/lg-buddy-production-canary')
    try:
        releases = release_history()
        candidate = api(f'repos/{REPOSITORY}/releases/tags/{args.expected_tag}')
        if api(f'repos/{REPOSITORY}/commits/{args.expected_tag}')['sha'] != args.expected_commit:
            raise AssetError('Published candidate tag differs from the reviewed release commit')
        verify_release_metadata(candidate, expected)
        baseline = select_baseline(releases, expected.version)
        record['baseline'] = baseline['tag_name'] if baseline else None
        identity, package = download_release(baseline or candidate, work / 'download', args.format)
        record['initial_package'] = {'tag': identity.release_tag, 'commit': identity.commit,
                                     'sha256': digest(package)}
        if args.format == 'rpm':
            install_log = checked('dnf5', '--assumeyes', '--setopt=install_weak_deps=False', '--setopt=tsflags=', 'install', str(package))
        else:
            # Minimal Ubuntu excludes docs, including the release manifest.
            Path('/etc/dpkg/dpkg.cfg.d/zz-lg-buddy-canary-documentation').write_text(
                'path-include=/usr/share/doc/lg-buddy\npath-include=/usr/share/doc/lg-buddy/*\n')
            install_log = checked('apt-get', 'install', '-y', '--no-install-recommends', str(package),
                                  env={**os.environ, 'DEBIAN_FRONTEND': 'noninteractive'})
        (work / 'baseline-install.output').write_text(install_log + '\n')
        username = 'lg-native-canary'
        checked('useradd', '--create-home', '--shell', '/bin/bash', username)
        user = pwd.getpwnam(username)
        protocol = 'apply-rpm-update' if args.format == 'rpm' else 'apply-debian-update'
        sudoers.write_text(f'{username} ALL=(root) NOPASSWD: /usr/bin/lg-buddy {protocol} *\n')
        sudoers.chmod(0o440)
        checked('visudo', '--check', '--file', str(sudoers))
        checked('runuser', '-u', username, '--', '/usr/bin/lg-buddy', 'settings', 'set', 'updates.channel', 'prerelease')
        config = Path(user.pw_dir) / '.config/lg-buddy/config.env'
        token = config.parent / 'tvs/primary/access-token.json'
        token.parent.mkdir(parents=True, exist_ok=True)
        token.write_text('{"access_token":"retained-public-canary-token"}\n')
        token.chmod(0o600)
        for path in (token.parent.parent, token.parent, token):
            os.chown(path, user.pw_uid, user.pw_gid)
        binding = Path('/etc/lg-buddy/config-path')
        binding.parent.mkdir(exist_ok=True)
        binding.write_text(str(config) + '\n')
        retained = [config, token, binding]
        before = snapshot(retained)
        environment = {name: value for name, value in os.environ.items()
                       if name not in ('GH_TOKEN', 'GITHUB_TOKEN') and not name.startswith('LG_BUDDY_')}
        if baseline:
            os.chown(work, user.pw_uid, user.pw_gid)
            output = work / 'native-update.output'
            checked('runuser', '-u', username, '--', 'dbus-run-session', '--', 'xvfb-run', '-a',
                    'python3', str(SCRIPTS / 'test-debian-update.py'), '--drive', 'yes',
                    '--version', expected.version, '--drive-output', str(output), env=environment)
            text = output.read_text()
            assert f'Current: {identity.version} ({identity.channel}, commit {identity.commit})' in text, text
            assert f'Target: {expected.version} ({expected.channel}, commit {expected.commit})' in text, text
            record['journey'] = 'prerelease-to-' + ('stable' if expected.channel == 'stable' else 'prerelease')
        else:
            # The first native release has no authentic older package to update.
            # Qualify its public installation, and record the missing journey.
            assert identity == expected
            record['unqualified'] = 'No earlier published native prerelease; an update journey is not yet possible'
        assert snapshot(retained) == before, 'public update changed retained user data'
        for binary in ('lg-buddy', 'lg-buddy-gui'):
            text = checked('/usr/bin/' + binary, '--version')
            assert text == (f'lg-buddy {expected.version}\nversion: {expected.version}\n'
                            f'channel: {expected.channel}\ncommit: {expected.commit}'), text
        record['installed_native_version'] = verify_installed_package(args.format)
        assert record['installed_native_version'] == expected.version.replace('-', '~', 1) + '-1'
        check = checked('runuser', '-u', username, '--', '/usr/bin/lg-buddy', 'updates', 'check', env=environment)
        assert 'status: up to date' in check and f'latest: {expected.version} ({expected.channel})' in check, check
        (work / 'candidate-update-check.output').write_text(check + '\n')
        observation = record_responses(recorder=GitHubRecorder(api_root='https://api.github.com',
                                      token=os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN')),
                                      repository=REPOSITORY, tag=args.expected_tag,
                                      target=expected.target, packages=True)
        (work / 'github-responses.json').write_text(json.dumps(observation, indent=2) + '\n')
        record['outcome'] = 'passed' if baseline else 'bootstrap'
    finally:
        sudoers.unlink(missing_ok=True)
        (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record))


if __name__ == '__main__':
    main()
