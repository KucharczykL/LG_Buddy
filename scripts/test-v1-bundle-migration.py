#!/usr/bin/env python3
"""Upgrade the pinned published v1.8 bundle before installed GUI startup."""

import argparse
import importlib
import json
import os
import subprocess
import tarfile
from pathlib import Path

from release_assets import digest, expected_identity
from release_bundle_manifest import validate_archive
from release_promotion import SemVer

SCRIPTS = Path(__file__).resolve().parent
PREVIOUS_COMMIT = 'ed0b740ee3873177682efe2d1f6ae7930e847aef'
PREVIOUS_DIGEST = 'c9e19ce2c2a265bda253180096c1babc772a93d980f1080436b321c638d6b9a4'


def run(command, environment, log):
    with log.open('wb') as output:
        subprocess.run(command, env=environment, stdout=output, stderr=subprocess.STDOUT,
                       check=True, timeout=180)


def snapshot(path):
    facts = path.stat()
    return path.read_bytes(), facts.st_uid, facts.st_gid, facts.st_mode & 0o7777


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--previous-archive', type=Path, required=True)
    candidate = parser.add_mutually_exclusive_group(required=True)
    candidate.add_argument('--candidate-archive', type=Path)
    candidate.add_argument('--published', action='store_true')
    parser.add_argument('--expected-tag', required=True)
    parser.add_argument('--expected-commit', required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() == 0:
        parser.error('run the isolated legacy upgrade as a normal user')
    assert digest(args.previous_archive) == PREVIOUS_DIGEST
    previous = validate_archive(args.previous_archive)
    expected = expected_identity(args.expected_tag, args.expected_commit)
    assert previous.release_tag == 'v1.8.0' and previous.commit == PREVIOUS_COMMIT
    assert SemVer.parse(expected.version) > SemVer.parse(previous.version)
    work = args.work_dir.resolve()
    work.mkdir(parents=True, exist_ok=False)
    if args.published:
        public = importlib.import_module('test-production-package-canary')
        release = public.api(f'repos/{public.REPOSITORY}/releases/tags/{args.expected_tag}')
        observed, _ = public.download_release(release, work / 'public', 'debian')
        assert observed == expected
        args.candidate_archive = work / 'public' / public.asset_names(expected.version)[0]
    assert validate_archive(args.candidate_archive) == expected
    bundles = []
    for name, archive in (('previous', args.previous_archive), ('candidate', args.candidate_archive)):
        destination = work / name
        destination.mkdir()
        with tarfile.open(archive) as bundle:
            bundle.extractall(destination, filter='data')
        roots = list(destination.iterdir())
        assert len(roots) == 1 and roots[0].is_dir()
        bundles.append(roots[0])
    home, prefix = work / 'home', work / 'root'
    (home / 'Desktop').mkdir(parents=True)
    prefix.mkdir()
    environment = {**os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(home / '.config'),
                   'XDG_CACHE_HOME': str(home / '.cache'), 'LG_BUDDY_INSTALL_ROOT': str(prefix),
                   'LG_BUDDY_SUDO_CMD': 'none', 'LG_BUDDY_NONINTERACTIVE': '1',
                   'LG_BUDDY_SKIP_SYSTEMD_ACTIONS': '1', 'LG_BUDDY_SKIP_PIP_INSTALL': '1',
                   'LG_BUDDY_TV_IP': '127.0.0.2', 'LG_BUDDY_TV_MAC': '02:00:00:00:01:36',
                   'LG_BUDDY_INPUT': 'HDMI_3', 'LG_BUDDY_TV_PLATFORM': 'lg_webos',
                   'LG_BUDDY_SCREEN_BACKEND': 'auto'}
    config = home / '.config/lg-buddy/config.env'
    config.parent.mkdir(parents=True)
    config.write_text('''# retained v1.8 preference sentinel
tvs_primary_ip=127.0.0.2
tvs_primary_mac=02:00:00:00:01:36
tvs_primary_input=HDMI_3
tvs_primary_platform=lg_webos
screen_backend=auto
screen_idle_blank=disabled
screen_idle_timeout=37
screen_restore_policy=aggressive
updates_channel=prerelease
updates_auto_check=disabled
system_sleep_wake_policy=disabled
''')
    config.chmod(0o640)
    token = config.parent / 'tvs/primary/access-token.json'
    token.parent.mkdir(parents=True, exist_ok=True)
    token.write_text('{"access_token":"retained-v1.8-credential"}\n')
    token.chmod(0o600)
    # Seed the supported retained profile so the historical installer deploys
    # its real payload without opening fresh pairing or contacting a TV.
    run(['bash', str(bundles[0] / 'install.sh')], environment, work / 'previous-install.output')
    for name in ('lg-buddy', 'lg-buddy-gui'):
        observed = subprocess.check_output([str(prefix / 'usr/bin' / name), '--version'], text=True)
        assert 'version: 1.8.0\n' in observed and f'commit: {PREVIOUS_COMMIT}\n' in observed
    # v1.8 no longer repairs Python environments. Seed its supported retained
    # 1.x migration input after deployment, rather than inventing a healthy
    # retired Python stack or contacting a TV just to create this test profile.
    config.write_bytes(config.read_bytes().replace(b'tvs_primary_platform=lg_webos', b'tvs_primary_platform=bscpylgtv')
                       .replace(b'screen_backend=auto', b'screen_backend=swayidle'))
    legacy_environment = prefix / 'usr/bin/LG_Buddy_PIP'
    legacy_environment.mkdir(exist_ok=True)
    (legacy_environment / 'retained-v1-marker').write_text('obsolete app environment\n')
    before, credential = snapshot(config), snapshot(token)
    run(['bash', str(bundles[1] / 'install.sh'), '--upgrade'], environment, work / 'candidate-upgrade.output')
    assert snapshot(config) == before and snapshot(token) == credential
    assert not legacy_environment.exists(), 'payload upgrade retained the obsolete app environment'
    for name in ('lg-buddy', 'lg-buddy-gui'):
        observed = subprocess.check_output([str(prefix / 'usr/bin' / name), '--version'], text=True)
        assert f'version: {expected.version}\n' in observed and f'commit: {expected.commit}\n' in observed
    run(['/usr/bin/python3', str(SCRIPTS / 'test-gui-startup-migration.py'), '--isolated-prefix', str(prefix),
         '--config', str(config), '--token', str(token), '--evidence', str(work / 'first-start')],
        environment, work / 'first-start.output')
    (work / 'result.json').write_text(json.dumps({'outcome': 'passed', 'baseline': previous.release_tag,
                                                'baseline_commit': previous.commit, 'candidate': expected.release_tag,
                                                'candidate_commit': expected.commit,
                                                'candidate_transport': 'public GitHub' if args.published else 'local verified bundle',
                                                'boundary': 'published v1.8 baseline; isolated prefix, skipped historical pip/service activation; real candidate GUI startup, no daemon'}, indent=2) + '\n')
    print('Published v1.8 bundle upgrade and installed first-start migration passed.')


if __name__ == '__main__':
    main()
