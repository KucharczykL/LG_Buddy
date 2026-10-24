#!/usr/bin/env python3
"""Stage the verified release payload shared by Debian and RPM packaging."""

from __future__ import annotations

import re
import tarfile
from collections.abc import Callable
from pathlib import Path

from release_bundle_manifest import ReleaseIdentity, validate_archive


RECIPE = Path(__file__).resolve().parent.parent / "packaging/common"
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


def put(root: Path, path: str, content: bytes, mode: int = 0o644) -> None:
    destination = root / path
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(content)
    destination.chmod(mode)


def stage_payload(
    archive: Path, root: Path, *, license_path: str,
    version_validator: Callable[[str], object],
) -> ReleaseIdentity:
    identity = validate_archive(archive)
    if identity.target != RUNTIME_TARGET or identity.gui_target != GUI_TARGET:
        raise PackageError("Native delivery requires the musl runtime and GNU GUI for x86_64")
    version_validator(identity.version)
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
        "LICENSE": (license_path, 0o644),
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
