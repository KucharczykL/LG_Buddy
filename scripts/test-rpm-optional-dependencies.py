#!/usr/bin/env python3
"""Prove that a real optional ELF library cannot add native dependencies."""

from __future__ import annotations

import argparse
import io
import subprocess
import tarfile
import tempfile
from pathlib import Path

from build_rpm_package import build
from release_bundle_manifest import validate_archive


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--source-date-epoch", type=int, required=True)
    args = parser.parse_args()
    identity = validate_archive(args.archive)
    with tempfile.TemporaryDirectory(prefix="lg-buddy-rpm-optional-") as directory:
        work = Path(directory)
        archive = work / args.archive.name
        optional_path = "docs/kwin/prebuilt/dependency-filter-fixture.so"
        # libstdc++ has real ELF dependencies/provides absent from the Rust/GTK
        # binary pair. It is test input only, never a published package asset.
        library = Path("/usr/lib64/libstdc++.so.6").read_bytes()
        with tarfile.open(args.archive, "r:gz") as original, tarfile.open(archive, "w:gz") as fixture:
            for member in original:
                fixture.addfile(member, original.extractfile(member) if member.isfile() else None)
            entry = tarfile.TarInfo(f"lg-buddy-{identity.version}-{identity.target}/{optional_path}")
            entry.size = len(library)
            entry.mode = 0o644
            fixture.addfile(entry, io.BytesIO(library))
        package = build(archive, work / "output", args.source_date_epoch)
        requires = subprocess.check_output(["rpm", "-qp", "--requires", str(package)], text=True)
        provides = subprocess.check_output(["rpm", "-qp", "--provides", str(package)], text=True)
        files = subprocess.check_output(["rpm", "-qpl", str(package)], text=True).splitlines()
        assert "/usr/lib/lg-buddy/" + optional_path.removeprefix("docs/") in files
        assert "libgtk-4.so.1" in requires, requires
        assert "libstdc++" not in requires + provides, requires + provides
    print("Optional ELF dependency filtering passed; GTK ELF dependencies remain declared.")


if __name__ == "__main__":
    main()
