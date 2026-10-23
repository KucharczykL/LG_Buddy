# Debian package development

The local builder produces an `amd64` `.deb` from the verified release bundle,
following the [package ownership contract](package-ownership.md). Its baseline
is Ubuntu 24.04. This is the first delivery slice of
[#134](https://github.com/Staphylococcus/LG_Buddy/issues/134): package assembly
and isolated lifecycle checks. Published package assets, APT installation in
`lg-buddy updates install`, and real Ubuntu VM qualification remain pending.
The current application updater continues to show package-manager guidance.

## Build

Build on Ubuntu 24.04 with `python3`, `dpkg-dev`, `libgtk-4-1` and
`libadwaita-1-0` installed. The input is the existing release bundle; the builder
checks its manifest and both embedded binary identities before staging files.
It does not run the bundle installer or execute archive scripts. Unsafe paths,
links, duplicate members and missing required assets refuse packaging.

```bash
python3 scripts/build_debian_package.py \
  --archive dist/lg-buddy-2.0.0-beta.1-x86_64-unknown-linux-musl.tar.gz \
  --source-date-epoch "$SOURCE_DATE_EPOCH" \
  --output-dir dist
```

Set `SOURCE_DATE_EPOCH` to the release commit's Unix timestamp. Repeating the
build with the same input, recipe, baseline dependency database and timestamp
produces the same package bytes. `dpkg-shlibdeps` derives dependencies from the
binary pair; metadata also declares GTK >= 4.14, libadwaita >= 1.5, systemd and
the polkit authorization tools. Optional KWin sources/prebuilts and GNOME
extension assets are included without requiring those desktops or a compiler.
Python is a build/test dependency only.

The example produces `lg-buddy_2.0.0~beta.1-1_amd64.deb`. Stable `2.0.0` becomes
`2.0.0-1`; `--revision 2` permits a packaging-only revision of the same upstream
release. Prerelease identifiers must each contain letters only or a number,
separated by dots (`beta.2`, `rc.1`, `beta.2.ci`). Mixed identifiers such as
`alpha10`, hyphenated identifiers and build metadata are refused because their
native ordering could disagree with SemVer. Embedded version/channel/commit
identity remains the upstream identity, independent of the Debian revision.

## Install and retain configuration

Install the local package through APT:

```bash
sudo apt install ./dist/lg-buddy_2.0.0~beta.1-1_amd64.deb
lg-buddy
```

Installation reloads the machine unit database when available. It does not
enable/start services, run setup, scan user homes, migrate settings or select a
configuration. The installed application's existing onboarding owns activation.
Upgrades and reinstalls preserve activation policy, dynamic bindings, settings
and credentials. Existing daemon processes continue until their normal restart;
packaging does not restart the TV startup/shutdown oneshot.

An unowned overlapping payload, redirected ancestor or local base unit refuses
installation before unpacking. Follow the contract's explicit manual
bundle-to-package transition first; the package does not adopt bundle files or
delete local units to make setup pass.

To remove your session integration, first run
`systemctl --user disable --now LG_Buddy_screen.service LG_Buddy_update_check.timer`
as that user. Package scripts do not select a user or operate their session
manager. `sudo apt remove lg-buddy` stops/disables the package's machine services and
removes its payload. Both removal and `sudo apt purge lg-buddy` retain unowned
machine bindings/drop-ins and all user data. There are no package conffiles.
For an explicit machine reset, an administrator must inspect the selected
binding and the two system-service `config.conf` drop-ins, confirm which are
unmodified LG Buddy-generated files, and unlink only those identified files.
Preserve modified/foreign files for manual resolution. Never follow the config
pointer into a user's data. User-data reset is a separate operation by that user.

## Validation

`python3 scripts/test_build_debian_package.py` checks input/layout contracts
and native version ordering (the ordering check needs `dpkg`). CI builds twice
to compare bytes, then runs the real APT/dpkg lifecycle in a disposable Ubuntu
24.04 container. The lifecycle script deliberately requires root, Docker and
`LG_BUDDY_DISPOSABLE_PACKAGE_TEST=1`:

```bash
LG_BUDDY_DISPOSABLE_PACKAGE_TEST=1 bash scripts/test-debian-package.sh \
  /packages/lg-buddy_2.0.0~beta.1-1_amd64.deb \
  /packages/lg-buddy_2.0.0~beta.1-2_amd64.deb
```

The smoke checks conflicts, install, a local package upgrade, reinstall,
ownership/modes/checksums, binary identities, runtime linkage, installed GTK
launch and removal/purge retention. Current and stale configuration/credential
fixtures must remain byte-for-byte unchanged by root package operations.
Running-manager commands are recorded fakes; offline enablement-link removal
uses real `systemctl`. Real systemd activation, non-root startup conversion,
desktop behavior and reboot require
the separate Ubuntu VM qualification. A packaging-revision upgrade does not
qualify the future updater's newer-upstream-release acquisition path.
