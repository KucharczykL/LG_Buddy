#!/usr/bin/env python3
"""Build an amd64 Debian package from a verified release bundle on Ubuntu 24.04."""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import tarfile
import tempfile
from pathlib import Path

from release_bundle_manifest import ManifestError, ReleaseIdentity, validate_archive
from release_promotion import SemVer


RECIPE = Path(__file__).resolve().parent.parent / "packaging/debian"
SYSTEM_UNITS = ("LG_Buddy.service", "LG_Buddy_lifecycle.service")
USER_UNITS = (
    "LG_Buddy_screen.service", "LG_Buddy_update_check.service",
    "LG_Buddy_update_check.timer",
)
ICON = "io.github.staphylococcus.LGBuddy.svg"
POLICY = "io.github.staphylococcus.LGBuddy.setup.policy"
RUNTIME_TARGET = "x86_64-unknown-linux-musl"
GUI_TARGET = "x86_64-unknown-linux-gnu"


class PackageError(RuntimeError):
    pass


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


def put(root: Path, path: str, content: bytes, mode: int = 0o644) -> None:
    destination = root / path
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(content)
    destination.chmod(mode)


def stage_payload(archive: Path, root: Path) -> ReleaseIdentity:
    identity = validate_archive(archive)
    if identity.target != RUNTIME_TARGET or identity.gui_target != GUI_TARGET:
        raise PackageError("Debian delivery requires the musl runtime and GNU GUI for x86_64")
    debian_version(identity.version)
    prefix = f"lg-buddy-{identity.version}-{identity.target}/"
    fixed = {
        "lg-buddy": ("usr/bin/lg-buddy", 0o755),
        f"docs/lg-buddy-gui-{GUI_TARGET}": ("usr/bin/lg-buddy-gui", 0o755),
        "bin/LG_Buddy_Common": ("usr/lib/lg-buddy/common.sh", 0o755),
        "docs/setup-services.sh": ("usr/lib/lg-buddy/setup-services", 0o755),
        f"docs/{POLICY}": (f"usr/share/polkit-1/actions/{POLICY}", 0o644),
        f"docs/{ICON}": (f"usr/share/icons/hicolor/scalable/apps/{ICON}", 0o644),
        "LG_Buddy_Brightness.desktop": ("usr/share/applications/io.github.staphylococcus.LGBuddy.desktop", 0o644),
        "README.md": ("usr/share/doc/lg-buddy/README.md", 0o644),
        "LICENSE": ("usr/share/doc/lg-buddy/copyright", 0o644),
        "release-manifest.json": ("usr/share/doc/lg-buddy/release-manifest.json", 0o644),
        "systemd/lg_buddy.conf": ("usr/lib/tmpfiles.d/lg_buddy.conf", 0o644),
    }
    for scope, units in (("system", SYSTEM_UNITS), ("user", USER_UNITS)):
        for unit in units:
            fixed[f"systemd/{unit}"] = (f"usr/lib/systemd/{scope}/{unit}", 0o644)
    required = set(fixed) | {
        "docs/kwin/setup.sh", "docs/kwin/build.sh",
        "docs/kwin/source/CMakeLists.txt", "docs/kwin/source/main.cpp",
        "docs/kwin/source/metadata.json",
        "docs/gnome-shell/lg-buddy-brightness@staphylococcus.github.io/metadata.json",
        "docs/gnome-shell/lg-buddy-brightness@staphylococcus.github.io/extension.js",
    }
    seen: set[str] = set()
    regular: set[str] = set()
    size = 0
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle:
            name = member.name.rstrip("/")
            if (
                name in seen or len(seen) >= 4096
                or not re.fullmatch(r"[A-Za-z0-9_+./@-]+", name)
                or any(part in ("", ".", "..") for part in name.split("/"))
                or (name != prefix[:-1] and not name.startswith(prefix))
                or not (member.isdir() or member.isfile())
            ):
                raise PackageError(f"unsafe or duplicate bundle member: {member.name}")
            seen.add(name)
            if member.isdir():
                continue
            regular.add(name)
            size += member.size
            if member.size > 128 * 1024 * 1024 or size > 512 * 1024 * 1024:
                raise PackageError("bundle payload exceeds package input limits")
            relative = name[len(prefix):]
            destinations: list[tuple[str, int]] = []
            if relative in fixed:
                destinations.append(fixed[relative])
            if relative.startswith("systemd/"):
                destinations.append(("usr/lib/lg-buddy/setup/" + relative, 0o644))
            elif relative.startswith("docs/kwin/"):
                mode = 0o755 if relative in ("docs/kwin/setup.sh", "docs/kwin/build.sh") else 0o644
                destinations.append(("usr/lib/lg-buddy/" + relative[5:], mode))
            elif relative.startswith("docs/gnome-shell/"):
                destinations.append(("usr/lib/lg-buddy/" + relative[5:], 0o644))
            elif relative.startswith("docs/") and (relative not in fixed or relative == f"docs/{ICON}"):
                destinations.append(("usr/share/doc/lg-buddy/" + relative, 0o644))
            if destinations:
                stream = bundle.extractfile(member)
                assert stream is not None
                content = stream.read()
                for destination, mode in destinations:
                    put(root, destination, content, mode)
    missing = sorted(prefix + path for path in required if prefix + path not in regular)
    if missing:
        raise PackageError(f"required package input missing: {missing[0]}")
    put(root, "usr/lib/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle",
        (RECIPE / "LG_Buddy_lifecycle").read_bytes(), 0o755)
    return identity


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
        output = output_dir / f"lg-buddy_{version}_amd64.deb"
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
