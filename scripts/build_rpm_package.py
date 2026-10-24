#!/usr/bin/env python3
"""Build an x86_64 RPM from a verified release bundle on Fedora 43."""

from __future__ import annotations

import argparse
import os
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

from package_payload import PackageError, stage_payload
from release_bundle_manifest import ManifestError
from release_promotion import SemVer


RECIPE = Path(__file__).resolve().parent.parent / "packaging/rpm"
LICENSE = "usr/share/licenses/lg-buddy/LICENSE"


def rpm_version(version: str, revision: int = 1) -> str:
    parsed = SemVer.parse(version)
    prerelease = ".".join(parsed.prerelease)
    # RPM compares numeric segments above alphabetic segments. In particular,
    # alpha.ci sorts below alpha.1, contrary to SemVer; refuse that spelling.
    if parsed.build or (prerelease and not re.fullmatch(
        r"(?:alpha|beta|rc)(?:\.(?:0|[1-9][0-9]*)(?:\.ci)?)?", prerelease
    )):
        raise PackageError(
            "RPM releases require alpha, beta or rc, optionally .N and then .ci, "
            "and no build metadata"
        )
    if revision < 1:
        raise PackageError("RPM release must be a positive integer")
    return version.replace("-", "~", 1) + f"-{revision}"


def render_spec(root: Path, version: str) -> str:
    upstream, revision = version.rsplit("-", 1)
    files = sorted(path for path in root.rglob("*") if path.is_file())
    inventory = "\n".join("/" + str(path.relative_to(root)) for path in files)
    pre = (RECIPE / "pre.in").read_text().replace("@PAYLOAD@", inventory)
    entries = []
    owned_roots = ("usr/lib/lg-buddy", "usr/share/doc/lg-buddy", "usr/share/licenses/lg-buddy")
    for path in sorted(root.rglob("*")):
        relative = str(path.relative_to(root))
        if path.is_dir():
            if any(relative == prefix or relative.startswith(prefix + "/") for prefix in owned_roots):
                entries.append("%dir /" + relative)
        elif relative == LICENSE:
            entries.append("%license /" + relative)
        elif relative.startswith("usr/share/doc/lg-buddy/"):
            entries.append("%doc /" + relative)
        else:
            entries.append("/" + relative)
    spec = (RECIPE / "lg-buddy.spec.in").read_text()
    replacements = {
        "@VERSION@": upstream, "@RELEASE@": revision,
        "@PAYLOAD_ROOT@": shlex.quote(str(root)), "@FILES@": "\n".join(entries),
        "@PRE@": pre.replace("%", "%%"),
    }
    for script in ("post", "preun", "postun"):
        replacements[f"@{script.upper()}@"] = (RECIPE / script).read_text().replace("%", "%%")
    # Substitute the trusted template once, without interpreting marker-looking
    # archive filenames introduced by a replacement.
    return re.sub(
        "|".join(map(re.escape, replacements)),
        lambda match: replacements[match[0]], spec,
    )


def build(archive: Path, output_dir: Path, epoch: int, revision: int = 1) -> Path:
    if epoch < 0:
        raise PackageError("SOURCE_DATE_EPOCH must be nonnegative")
    with tempfile.TemporaryDirectory(prefix="lg-buddy-rpm-") as temporary:
        work = Path(temporary)
        if "%" in str(work):
            raise PackageError("RPM build directory must not contain macro characters")
        root = work / "payload"
        identity = stage_payload(
            archive, root, license_path=LICENSE, version_validator=rpm_version,
        )
        version = rpm_version(identity.version, revision)
        spec = work / "lg-buddy.spec"
        spec.write_text(render_spec(root, version))
        for path in sorted(root.rglob("*")):
            if path.is_dir():
                path.chmod(0o755)
            os.utime(path, (epoch, epoch))
        root.chmod(0o755)
        os.utime(root, (epoch, epoch))
        command = ["rpmbuild", "-bb", "--buildroot", str(work / "buildroot")]
        for definition in (
            f"_topdir {work}", "_buildhost lg-buddy", "_rpmformat 6",
            "use_source_date_epoch_as_buildtime 1",
            "build_mtime_policy clamp_to_source_date_epoch",
            "_binary_filedigest_algorithm 8", "_binary_payload w9.gzdio",
            "_smp_nthreads_max 1",
        ):
            command.extend(("--define", definition))
        subprocess.run(
            [*command, str(spec)], check=True,
            env={**os.environ, "SOURCE_DATE_EPOCH": str(epoch)},
        )
        name = f"lg-buddy-{version}.x86_64.rpm"
        candidate = work / "RPMS/x86_64" / name
        output_dir.mkdir(parents=True, exist_ok=True)
        output = output_dir / name
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
        "--source-date-epoch", type=int, default=os.environ.get("SOURCE_DATE_EPOCH"),
        required="SOURCE_DATE_EPOCH" not in os.environ,
    )
    args = parser.parse_args()
    try:
        output = build(args.archive.resolve(), args.output_dir.resolve(), args.source_date_epoch, args.revision)
    except (PackageError, ManifestError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"RPM package build failed: {error}\n")
    print(output)


if __name__ == "__main__":
    main()
