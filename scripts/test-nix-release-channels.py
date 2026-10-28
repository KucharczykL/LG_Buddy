#!/usr/bin/env python3
"""Build real Nix channel packages and verify installed/embedded identities.

Version fixtures are materialized before evaluation, avoiding import-from-
derivation and keeping cross-architecture flake evaluation build-free.
"""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


def output(*args):
    return subprocess.check_output(args, text=True).strip()


def fixture(root, destination, version):
    for relative in output("git", "-C", str(root), "ls-files", "-z").split("\0"):
        source = root / relative
        if not relative or not source.is_file():
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    for name in ("lg-buddy", "lg-buddy-gui"):
        manifest = destination / "crates" / name / "Cargo.toml"
        manifest.write_text(re.sub(
            r'(?m)^version = "[^"]+"', f'version = "{version}"',
            manifest.read_text(), count=1,
        ))
    lock = destination / "Cargo.lock"
    text = lock.read_text()
    for name in ("lg-buddy", "lg-buddy-gui"):
        text = re.sub(
            r'(\[\[package\]\]\nname = "' + name + r'"\nversion = ")[^"]+',
            lambda match: match[1] + version, text,
        )
    lock.write_text(text)


def check_package(package, version, channel, commit, target, temporary, objcopy):
    expected = {"version": version, "channel": channel, "commit": commit}
    for launcher, executable in [
        ("lg-buddy", "lg-buddy"),
        ("lg-buddy-gui", ".lg-buddy-gui-wrapped"),
    ]:
        result = output(str(package / "bin" / launcher), "--version")
        lines = result.splitlines()
        assert lines[0] == f"lg-buddy {version}", result
        observed = dict(line.split(": ", 1) for line in lines[1:])
        assert observed == expected, (launcher, observed, expected)

        record = temporary / "identity"
        record.unlink(missing_ok=True)
        subprocess.run([
            str(objcopy), "--dump-section", f".lg_buddy.identity={record}",
            package / "bin" / executable, temporary / "executable",
        ], check=True)
        content = record.read_bytes()
        prefix = b"LG_BUDDY_RELEASE_IDENTITY_V1\0"
        suffix = b"\0LG_BUDDY_RELEASE_IDENTITY_END\0"
        assert content.startswith(prefix) and content.endswith(suffix)
        identity = json.loads(content[len(prefix):-len(suffix)])
        assert {key: identity[key] for key in expected} == expected, identity
        assert identity["schema_version"] == 1
        assert identity["target"] == target
        assert identity["release_tag"] == f"v{version}"
    print(f"Verified {channel}: CLI, wrapped GUI and embedded identity.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cores", default="2")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    system = output("nix", "eval", "--impure", "--raw", "--expr", "builtins.currentSystem")
    with tempfile.TemporaryDirectory(prefix="lg-buddy-nix-channels-") as directory:
        temporary = Path(directory)
        stable = temporary / "stable"
        prerelease = temporary / "prerelease"
        fixture(root, stable, "9.8.7")
        fixture(root, prerelease, "9.8.7-beta.1")
        for source in (stable, prerelease):
            subprocess.run(["nix", "flake", "check", "--no-build", "--all-systems",
                            f"path:{source}"], check=True)
            # The development output stays development even with a beta Cargo
            # version, and the backwards-compatible aliases select that output.
            development = json.loads(output("nix", "eval", "--json", "--apply",
                'p: { channel = p.releaseChannel; release = p.LG_BUDDY_RELEASE_VERSION; }',
                f"path:{source}#packages.{system}.lg-buddy-dev"))
            assert development == {"channel": "dev", "release": ""}
            default = output("nix", "eval", "--raw", f"path:{source}#packages.{system}.default.outPath")
            explicit = output("nix", "eval", "--raw", f"path:{source}#packages.{system}.lg-buddy-dev.outPath")
            assert default == explicit
        tools = json.loads(output("nix", "build", "--json", "--out-link", str(temporary / "tools"),
                                  "--inputs-from", str(root), "nixpkgs#binutils"))
        objcopy = Path(tools[0]["outputs"]["out"]) / "bin/objcopy"
        for label, source, attribute, expected_channel in [
            ("dev", str(root), "lg-buddy-dev", "dev"),
            ("stable", f"path:{stable}", "lg-buddy-stable", "stable"),
            ("prerelease", f"path:{prerelease}", "lg-buddy-prerelease", "prerelease"),
        ]:
            selection = f"{source}#packages.{system}.{attribute}"
            evaluated = json.loads(output("nix", "eval", "--json", "--apply",
                'p: { inherit (p) version; commit = p.LG_BUDDY_BUILD_COMMIT; }', selection))
            built = json.loads(output("nix", "build", "--json", "--print-build-logs",
                "--cores", args.cores, "--max-jobs", "2", "--out-link", str(temporary / label), selection))
            package = Path(built[0]["outputs"]["out"])
            print(f"Checking {label} package: {package}", flush=True)
            target = output("nix", "eval", "--raw", "--impure", "--expr",
                f'(builtins.getFlake "{root}").inputs.nixpkgs.legacyPackages.{system}.stdenv.hostPlatform.rust.rustcTarget')
            check_package(package, evaluated["version"], expected_channel, evaluated["commit"],
                          target, temporary, objcopy)
            subprocess.run(["bash", root / "scripts/test-nix-update-ownership.sh", package], check=True)
        # The prerelease stream can point at stable after a stable promotion.
        aligned = output("nix", "eval", "--raw", f"path:{stable}#packages.{system}.lg-buddy-prerelease.outPath")
        released = output("nix", "eval", "--raw", f"path:{stable}#packages.{system}.lg-buddy-stable.outPath")
        assert aligned == released
        # A beta source must never offer a stable package.
        rejected = subprocess.run(["nix", "eval", "--raw",
            f"path:{prerelease}#packages.{system}.lg-buddy-stable.outPath"], capture_output=True)
        assert rejected.returncode != 0


if __name__ == "__main__":
    main()
