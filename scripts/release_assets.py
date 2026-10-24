#!/usr/bin/env python3
"""Verify the complete bundle/DEB/RPM asset set without executing package code."""

from __future__ import annotations

import argparse
import hashlib
import io
import os
import re
import subprocess
import tarfile
from pathlib import Path

from build_debian_package import asset_name as debian_asset_name, debian_version
from build_rpm_package import asset_name as rpm_asset_name, rpm_version
from package_payload import GUI_TARGET, RUNTIME_TARGET, PackageError
from release_bundle_manifest import (
    ManifestError, ReleaseIdentity, parse_manifest, validate_archive,
    validate_embedded_binary_matches, validate_manifest,
)
from release_promotion import PromotionError


class AssetError(RuntimeError):
    pass


def asset_names(version: str, target: str = RUNTIME_TARGET) -> tuple[str, ...]:
    if target != RUNTIME_TARGET:
        raise AssetError(f"Unsupported release target: {target}")
    return (
        f"lg-buddy-{version}-{target}.tar.gz",
        debian_asset_name(version),
        rpm_asset_name(version),
        "sha256sums.txt",
    )


def expected_identity(tag: str, commit: str) -> ReleaseIdentity:
    if not tag.startswith('v') or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise AssetError("Expected a release tag and full lowercase commit SHA")
    version = tag[1:]
    asset_names(version)  # Enforce both native version mappings.
    return ReleaseIdentity(tag, version, "prerelease" if '-' in version else "stable",
                           RUNTIME_TARGET, commit, GUI_TARGET)


def decode(package: Path, command: list[str], limit: int) -> bytes:
    with package.open('rb') as stream:
        process = subprocess.Popen(command, stdin=stream, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL,
                                   env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                                        "LC_ALL": "C", "HOME": "/root"})
        try:
            assert process.stdout is not None
            content = process.stdout.read(limit + 1)
            if len(content) > limit:
                raise AssetError(f"Native decoder exceeded its output limit: {command[0]}")
            if process.wait(timeout=30) != 0:
                raise AssetError(f"Native decoder failed: {command[0]}")
            return content
        finally:
            if process.stdout is not None:
                process.stdout.close()
            if process.poll() is None:
                process.kill()
            process.wait()


def payload_identity(data: bytes, expected: ReleaseIdentity) -> set[str]:
    seen: set[str] = set()
    found: set[str] = set()
    required = {
        "usr/bin/lg-buddy": expected.target,
        "usr/bin/lg-buddy-gui": GUI_TARGET,
        "usr/share/doc/lg-buddy/release-manifest.json": None,
    }
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:') as payload:
        for member in payload:
            name = member.name.removeprefix('./').rstrip('/')
            if name in ('', '.') and member.isdir():
                continue
            if (len(seen) >= 4096 or name in seen
                or any(part in ('', '.', '..') for part in name.split('/'))
                or not (name == 'usr' or name.startswith('usr/'))
                or name == 'usr/lib/lg-buddy/config-path'
                or not (member.isfile() or member.isdir())
                or member.uid != 0 or member.gid != 0
                or member.mode not in (0o644, 0o755)
                or (member.isdir() and member.mode != 0o755)
                or member.size > 128 * 1024 * 1024):
                raise AssetError(f"Unsafe native payload member: {member.name}")
            seen.add(name)
            if name in required:
                if not member.isfile():
                    raise AssetError(f"Release identity member is not a file: {name}")
                stream = payload.extractfile(member)
                assert stream is not None
                content = stream.read()
                target = required[name]
                if target is None:
                    if validate_manifest(parse_manifest(content)) != expected:
                        raise AssetError("Native manifest disagrees with the release")
                else:
                    if member.mode != 0o755:
                        raise AssetError(f"Native executable mode is not 0755: {name}")
                    validate_embedded_binary_matches(expected, content, target=target, label=name)
                found.add(name)
    if found != required.keys():
        raise AssetError("Native package must contain both binaries and the release manifest")
    return seen


def verify_debian_control(data: bytes) -> None:
    """Match the installed updater's bounded, flat control-archive contract."""
    seen = set()
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:') as control:
        for member in control:
            name = member.name.removeprefix('./')
            if name in ('', '.') and member.isdir():
                continue
            if (len(seen) >= 64 or name in seen or not member.isfile()
                or name in ('', '.', '..') or '/' in name or '\\' in name or '\0' in name
                or len(name.encode('utf-8')) > 4096
                or (name == 'conffiles' and member.size != 0)):
                raise AssetError('Unsafe Debian control metadata or package-owned configuration')
            seen.add(name)


def verify_package(package: Path, expected: ReleaseIdentity) -> None:
    names = asset_names(expected.version)
    if package.name == names[1]:
        control = decode(package, ['dpkg-deb', '--ctrl-tarfile', str(package)], 64 * 1024)
        verify_debian_control(control)
        metadata = decode(package, ['dpkg-deb', '--field', str(package),
                                   'Package', 'Version', 'Architecture'], 64 * 1024)
        wanted = (f"Package: lg-buddy\nVersion: {debian_version(expected.version)}\n"
                  "Architecture: amd64\n").encode()
        if metadata != wanted:
            raise AssetError("Debian package name, version or architecture disagrees with the release")
        data = decode(package, ['dpkg-deb', '--fsys-tarfile', str(package)], 512 * 1024 * 1024)
        payload_identity(data, expected)
    elif package.name == names[2]:
        metadata = decode(package, ['rpm', '--query', '--package', '--queryformat',
                                   '%{NAME}\n%{EPOCHNUM}\n%{VERSION}-%{RELEASE}\n%{ARCH}\n', '-'], 64 * 1024)
        if metadata != f"lg-buddy\n0\n{rpm_version(expected.version)}\nx86_64\n".encode():
            raise AssetError("RPM name, epoch, version or architecture disagrees with the release")
        inventory = decode(package, ['rpm', '--query', '--package', '--queryformat',
                                    '[%{FILENAMES}\t%{FILEFLAGS}\n]', '-'], 1024 * 1024)
        paths = set()
        for line in inventory.decode().splitlines():
            path, flags = line.split('\t')
            if not path.startswith('/usr/') or int(flags) & ~(2 | 128) or path[1:] in paths:
                raise AssetError("Unsafe or duplicate RPM file inventory")
            paths.add(path[1:])
        data = decode(package, ['rpm2archive', '-n', '-'], 512 * 1024 * 1024)
        if payload_identity(data, expected) != paths:
            raise AssetError("RPM header and payload paths disagree")
    else:
        raise AssetError(f"Unexpected native package filename: {package.name}")


def digest(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_checksums(path: Path, names: tuple[str, ...]) -> dict[str, str]:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 64 * 1024:
        raise AssetError("Expected a bounded regular checksum asset")
    recorded = {}
    for line in path.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64}) [ *](?:\./)?([^/]+)", line)
        if match is None or match[2] in recorded:
            raise AssetError("Malformed or duplicate checksum entry")
        recorded[match[2]] = match[1]
    if set(recorded) != set(names):
        raise AssetError("Checksums must cover exactly the bundle, Debian package and RPM")
    return recorded


def verify_assets(directory: Path, expected: ReleaseIdentity, *, write_checksums: bool = False) -> list[Path]:
    names = asset_names(expected.version)
    # The bundle builder also leaves its expanded staging directory here.
    # Only top-level files are release assets; redirected entries still refuse.
    present = {path.name for path in directory.iterdir() if path.is_symlink() or not path.is_dir()}
    wanted = set(names[:-1] if write_checksums else names)
    if write_checksums:
        present.discard(names[-1])
    if present != wanted:
        raise AssetError(f"Release asset inventory differs: missing={sorted(wanted - present)}, unexpected={sorted(present - wanted)}")
    paths = [directory / name for name in names]
    for path in paths[:3]:
        if path.is_symlink() or not path.is_file() or path.stat().st_size > 64 * 1024 * 1024:
            raise AssetError(f"Expected a bounded regular release asset: {path.name}")
    checksums = paths[-1]
    if checksums.is_symlink():
        raise AssetError("Checksum asset must not be a symlink")
    if not write_checksums:
        recorded = read_checksums(checksums, names[:3])
        for path in paths[:3]:
            if digest(path) != recorded[path.name]:
                raise AssetError(f"Release asset checksum mismatch: {path.name}")
    if validate_archive(paths[0]) != expected:
        raise AssetError("Release bundle identity disagrees with the release commit")
    for package in paths[1:3]:
        verify_package(package, expected)
    if write_checksums:
        checksums.write_text(''.join(f"{digest(path)}  {path.name}\n" for path in paths[:3]))
    return paths


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dist-dir', type=Path, default=Path('dist'))
    parser.add_argument('--tag', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--write-checksums', action='store_true')
    parser.add_argument('--list-assets', action='store_true')
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--package', type=Path)
    args = parser.parse_args()
    try:
        expected = expected_identity(args.tag, args.commit)
        if args.package:
            if not args.archive or validate_archive(args.archive) != expected:
                raise AssetError('Single-package verification requires the matching release bundle')
            verify_package(args.package.resolve(), expected)
        else:
            paths = verify_assets(args.dist_dir.resolve(), expected, write_checksums=args.write_checksums)
            if args.list_assets:
                print('\n'.join(map(str, paths)))
    except (AssetError, ManifestError, PackageError, PromotionError, OSError,
            ValueError, tarfile.TarError, subprocess.SubprocessError) as error:
        parser.exit(1, f"Release asset verification failed: {error}\n")


if __name__ == '__main__':
    main()
