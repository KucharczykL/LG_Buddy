# Debian package development

The local builder produces an `amd64` `.deb` from the verified release bundle,
following the [package ownership contract](package-ownership.md). Its baseline
is Ubuntu 24.04. Package assembly, isolated lifecycle checks and the APT updater
adapter are implemented under
[#134](https://github.com/Staphylococcus/LG_Buddy/issues/134).
[Ubuntu VM qualification](debian-vm-validation.md) also passed for the reviewed
product source. Published package assets remain
[#136](https://github.com/Staphylococcus/LG_Buddy/issues/136); the adapter requires
a matching verified `.deb` on the selected release.

## Build

Build on Ubuntu 24.04 with `python3`, `dpkg-dev`, `libgtk-4-1` and
`libadwaita-1-0` installed. The input is the existing release bundle; the builder
checks its manifest and both embedded binary identities before staging files.
It does not run the bundle installer or execute archive scripts. Unsafe paths,
links, duplicate members and missing required assets refuse packaging.

```bash
python3 scripts/build_debian_package.py \
  --archive dist/lg-buddy-2.0.0-beta.3-x86_64-unknown-linux-musl.tar.gz \
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

The example produces `lg-buddy_2.0.0-beta.3-1_amd64.deb`. The filename uses
upstream SemVer because GitHub rewrites `~` on upload; native package metadata
retains `2.0.0~beta.3-1` for correct prerelease ordering. Stable `2.0.0` becomes
`2.0.0-1`; `--revision 2` permits a packaging-only revision of the same upstream
release. Supported prereleases are `alpha`, `beta` or `rc`, optionally followed
by `.N` and `.ci` (`beta.2`, `rc.1`, `beta.2.ci`). Other prerelease forms and
build metadata are refused because their native ordering could disagree with
SemVer, including letter-only examples such as `alpha.a.1` versus `alpha.aa`.
Embedded version/channel/commit identity remains the upstream identity,
independent of the Debian revision. Official updater assets use revision `1`;
later local packaging revisions are installed manually through APT.

## Integrated updates

`lg-buddy updates install` and the GUI share the saved channel, discovery,
confirmation and result verification. A healthy `lg-buddy` Debian installation
acquires `lg-buddy_<release-version>-1_amd64.deb`; bundles retain their existing
installer. Other owners, incomplete/held packages, conflicting ownership,
managed hosts and unsafe/read-only paths refuse automatic native installation.

Acquisition verifies the GitHub asset digest, published checksum, release tag
commit, native name/version/architecture and both embedded binary identities.
The package must leave dynamic configuration unowned. Before APT runs, the
installed runtime's private administrator-authorized entrypoint independently
checks the release and checksum, then copies the supplied bytes into protected
temporary storage under `/run/lg-buddy-package-updates`. It never accepts a
caller-selected package path or command. APT receives that protected file,
resolves dependencies and owns the transaction; removals, downgrades, held
package changes and unauthenticated dependencies are refused.

Success verifies the installed native package version, both file owners and
both binary identities. The GUI restarts through the existing handoff. An
interactive terminal update in a normal user's graphical session also launches
the installed GUI. Root/headless terminal updates leave GUI launch to the user.
An APT error after transaction start reports possible partial installation;
inspect and repair native package state before retrying. Installing a local
`.deb` does not create an APT repository or make `apt upgrade` discover releases.

## Install and retain configuration

Install the local package through APT:

```bash
sudo apt install ./dist/lg-buddy_2.0.0-beta.3-1_amd64.deb
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
  /packages/lg-buddy_2.0.0-beta.3-1_amd64.deb \
  /packages/lg-buddy_2.0.0-beta.3-2_amd64.deb
```

The smoke checks conflicts, install, a local package upgrade, reinstall,
ownership/modes/checksums, binary identities, runtime linkage, installed GTK
launch and removal/purge retention. Current and stale configuration/credential
fixtures must remain byte-for-byte unchanged by root package operations.
After the upgrade/reinstall checks, the smoke starts the installed monitor as
the stale config's non-root owner without a desktop/session bus. Only the two
retired selectors may change; preferences, file ownership/mode and credentials
must remain intact. Root lifecycle and foreground TV commands must refuse that
stale input without converting it. The converted profile is then retained
through removal and purge.
Running-manager commands are recorded fakes; offline enablement-link removal
uses real `systemctl`. Real systemd activation, non-root startup conversion,
desktop behavior and reboot are covered separately by the
[Ubuntu VM qualification](debian-vm-validation.md).
A packaging-revision upgrade does not
qualify the updater's newer-upstream-release acquisition path.

`scripts/test-debian-update.py` separately runs the installed application
through a real APT update and user GUI relaunch. Its `0.0.0` baseline uses the
existing GNU debug binaries with the debug-only GitHub transport fixture; the
candidate is the verified release package. The test checks root digest refusal,
declined confirmation, acquired package replacement and retained data. Only
HTTP transport and administrator authentication are substituted: a local GitHub
fixture and a narrowly scoped disposable NOPASSWD sudo rule are used. Release
builds cannot enable the transport feature. This does not qualify real polkit
prompts or a running desktop/system service manager.
