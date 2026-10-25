#!/usr/bin/env python3
"""Build an amd64 Debian package from a verified release bundle on Ubuntu 24.04."""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import tempfile
from pathlib import Path

from release_bundle_manifest import ManifestError, ReleaseIdentity
from release_promotion import SemVer
from package_payload import (
    GUI_TARGET, ICON, POLICY, SYSTEM_UNITS, USER_UNITS, RUNTIME_TARGET,
    PackageError, put, stage_payload as _stage_payload,
)


RECIPE = Path(__file__).resolve().parent.parent / "packaging/debian"


def debian_version(version: str, revision: int = 1) -> str:
    parsed = SemVer.parse(version)
    # General SemVer identifiers do not preserve their ordering in dpkg, even
    # when they contain only letters. Use the project's conventional channels.
    prerelease = ".".join(parsed.prerelease)
    if parsed.build or (prerelease and not re.fullmatch(
        r"(?:alpha|beta|rc)(?:\.(?:0|[1-9][0-9]*))?(?:\.ci)?", prerelease
    )):
        raise PackageError(
            "Debian releases require alpha, beta or rc, optionally followed "
            "by .N and .ci, and no build metadata"
        )
    if revision < 1:
        raise PackageError("Debian revision must be a positive integer")
    return version.replace("-", "~", 1) + f"-{revision}"


def asset_name(version: str, revision: int = 1) -> str:
    # GitHub normalizes '~' on upload; native metadata still retains it.
    portable = debian_version(version, revision).replace("~", "-")
    return f"lg-buddy_{portable}_amd64.deb"


def stage_payload(archive: Path, root: Path) -> ReleaseIdentity:
    return _stage_payload(
        archive, root, license_path="usr/share/doc/lg-buddy/copyright",
        version_validator=debian_version,
    )


def dependencies(root: Path, work: Path) -> str:
    # dpkg-shlibdeps consults the baseline's installed symbols/shlibs database.
    # Include both executables; a static runtime contributes no library packages.
    debian = work / "debian"
    debian.mkdir(exist_ok=True)
    (debian / "control").write_text("Source: lg-buddy\n\nPackage: lg-buddy\nArchitecture: amd64\n")
    result = subprocess.run(
        ["dpkg-shlibdeps", "-O",
         "-e" + str(root.relative_to(work) / "usr/bin/lg-buddy"),
         "-e" + str(root.relative_to(work) / "usr/bin/lg-buddy-gui")],
        cwd=work, check=True, text=True, stdout=subprocess.PIPE,
    )
    line = result.stdout.strip()
    if not line.startswith("shlibs:Depends=") or "\n" in line:
        raise PackageError("dpkg-shlibdeps did not produce a dependency list")
    generated = line.removeprefix("shlibs:Depends=")
    if not generated:
        raise PackageError("GTK executable has no native library dependencies")
    return ", ".join((generated, "libgtk-4-1 (>= 4.14)", "libadwaita-1-0 (>= 1.5)",
                      "apt", "systemd", "init-system-helpers", "polkitd", "pkexec"))


def build(archive: Path, output_dir: Path, epoch: int, revision: int = 1) -> Path:
    if epoch < 0:
        raise PackageError("SOURCE_DATE_EPOCH must be nonnegative")
    with tempfile.TemporaryDirectory(prefix="lg-buddy-deb-") as temporary:
        work = Path(temporary)
        root = work / "debian/lg-buddy"
        identity = stage_payload(archive, root)
        version = debian_version(identity.version, revision)
        depends = dependencies(root, work)
        files = sorted(path for path in root.rglob("*") if path.is_file())
        size = (sum(path.stat().st_size for path in files) + 1023) // 1024
        control = (RECIPE / "control.in").read_text().replace("@VERSION@", version)
        control = control.replace("@DEPENDS@", depends).replace("@SIZE@", str(size))
        put(root, "DEBIAN/control", control.encode())
        hashes = "".join(
            f"{hashlib.md5(path.read_bytes()).hexdigest()}  {path.relative_to(root)}\n"
            for path in files
        )
        put(root, "DEBIAN/md5sums", hashes.encode())
        inventory = "\n".join("/" + str(path.relative_to(root)) for path in files)
        preinst = (RECIPE / "preinst.in").read_text().replace("@PAYLOAD@", inventory)
        put(root, "DEBIAN/preinst", preinst.encode(), 0o755)
        for script in ("postinst", "prerm", "postrm"):
            put(root, f"DEBIAN/{script}", (RECIPE / script).read_bytes(), 0o755)
        for path in sorted(root.rglob("*")):
            if path.is_dir():
                path.chmod(0o755)
            os.utime(path, (epoch, epoch))
        root.chmod(0o755)
        os.utime(root, (epoch, epoch))
        output_dir.mkdir(parents=True, exist_ok=True)
        output = output_dir / asset_name(identity.version, revision)
        # Publish only a completed package. Build twice to the same output safely.
        candidate = work / output.name
        subprocess.run(
            ["dpkg-deb", "--root-owner-group", "--threads-max=1", "-Zxz",
             "--build", str(root), str(candidate)],
            check=True, env={**os.environ, "SOURCE_DATE_EPOCH": str(epoch)},
        )
        with tempfile.NamedTemporaryFile(dir=output_dir, prefix=".lg-buddy-", delete=False) as staged:
            staged_path = Path(staged.name)
            try:
                staged.write(candidate.read_bytes())
                staged.flush()
                staged_path.chmod(0o644)
                os.replace(staged_path, output)
            finally:
                staged_path.unlink(missing_ok=True)
        return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    parser.add_argument("--revision", type=int, default=1)
    parser.add_argument(
        "--source-date-epoch", type=int,
        default=os.environ.get("SOURCE_DATE_EPOCH"),
        required="SOURCE_DATE_EPOCH" not in os.environ,
    )
    args = parser.parse_args()
    try:
        output = build(
            args.archive.resolve(), args.output_dir.resolve(),
            args.source_date_epoch, args.revision,
        )
    except (PackageError, ManifestError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Debian package build failed: {error}\n")
    print(output)


if __name__ == "__main__":
    main()
