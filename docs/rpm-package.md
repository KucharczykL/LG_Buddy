# RPM package development

The local builder produces an `x86_64` RPM from the verified release bundle,
following the [package ownership contract](package-ownership.md). Its baseline
is Fedora 43. Package assembly, lifecycle checks and DNF updater integration
implement [#135](https://github.com/Staphylococcus/LG_Buddy/issues/135).
Real Fedora desktop qualification is recorded in
[rpm-vm-validation.md](rpm-vm-validation.md). Public package assets and
production canaries belong to
[#136](https://github.com/Staphylococcus/LG_Buddy/issues/136).

## Build

Build on Fedora 43 with `python3` and `rpm-build`. The shared Debian/RPM payload
validator checks the release manifest and both embedded binary identities,
rejects unsafe archive members, and stages only vendor files. It never executes
archive scripts or the bundle installer.

```bash
python3 scripts/build_rpm_package.py \
  --archive dist/lg-buddy-2.0.0-beta.3-x86_64-unknown-linux-musl.tar.gz \
  --source-date-epoch "$SOURCE_DATE_EPOCH" \
  --output-dir dist
```

Set `SOURCE_DATE_EPOCH` to the release commit's Unix timestamp. Repeating the
build with the same input, recipe, Fedora/RPM baseline and timestamp produces
the same package bytes. The builder fixes the build host, timestamps, digest
and compression settings using RPM's
[reproducible-build configuration](https://rpm.org/docs/6.0.x/man/rpmbuild-config.5).
It emits RPM v6 and preserves the already-built release binaries without
stripping or rewriting them.

The example produces `lg-buddy-2.0.0-beta.3-1.x86_64.rpm`. The filename uses
upstream SemVer because GitHub rewrites `~` on upload; native package metadata
retains `2.0.0~beta.3-1` for correct prerelease ordering. Stable `2.0.0` becomes
`2.0.0-1`; `--revision 2` makes a packaging-only release. Supported prereleases
are `alpha`, `beta` or `rc`, optionally followed by `.N` and then `.ci`.
Build metadata and other prerelease forms are refused. In particular,
`alpha.ci` without a number would sort below `alpha.1` in RPM, contrary to
SemVer. Native ordering tests cover every supported form. Both binaries retain
their upstream version/channel/commit identity, independent of the RPM release.

RPM generates actual ELF library dependencies and declares GTK >= 4.14,
libadwaita >= 1.5, the GLES renderer library, systemd, polkit and DNF5.
GTK loads GLES dynamically, so its provider is an explicit dependency even
when weak dependencies are disabled. Optional KWin payloads are excluded
from dependency/provides scanning so they do not require KWin, Qt or a compiler
on every desktop. Python is a build/test dependency only. The GPL license is
installed as `%license` under `/usr/share/licenses/lg-buddy/`.

## Install and retain configuration

```bash
sudo dnf5 install ./dist/lg-buddy-2.0.0-beta.3-1.x86_64.rpm
lg-buddy
```

Installation reloads the machine unit database when available. It does not
enable/start services, choose a desktop user, scan homes, select a configuration,
pair a TV, migrate settings or launch UI. The installed application's shared
onboarding owns activation. Upgrade and reinstall preserve activation policy,
settings, native credentials and dynamic configuration. Scriptlets do not
restart existing daemons or the TV startup/shutdown oneshot.

An unowned overlapping payload, another package's overlapping file, redirected
ancestor, local base unit or legacy binding refuses installation before
unpacking. Even an identical file owned by another package is refused. Use the
contract's explicit manual bundle-to-package transition first.

To remove session integration, first run
`systemctl --user disable --now LG_Buddy_screen.service LG_Buddy_update_check.timer`
as that user. `sudo dnf5 remove lg-buddy` stops/disables the package's machine
services and removes its vendor payload. It retains dynamic bindings, drop-ins,
settings, credentials, caches and user state. Package scripts never contact a
user's service manager.

RPM has no Debian-style purge operation. For an explicit machine reset, an
administrator must inspect the selected `/etc/lg-buddy/config-path` and
LG Buddy-generated system drop-ins, then remove only those verified generated
files. Preserve modified/foreign files for administrator resolution and never
follow the pointer into a user's data. User-data reset is a separate operation
by that user. Installing a local RPM does not configure a DNF repository.

## Updates

The installed GUI and `lg-buddy updates install` use the shared channel,
discovery and explicit confirmation flow. An RPM-owned installation selects
the matching `.rpm`, checks official release/tag/checksum metadata and both
embedded binary identities, then asks for ordinary administrator authorization.
The privileged helper independently re-fetches the release identity and digest;
caller-supplied paths, digests and command arguments cannot choose root work.

DNF5 stores an upgrade transaction first. LG Buddy checks that it selects the
exact newer local package, permits only dependency installs and same-package
upgrades/replacements, and refuses removals, downgrades, foreign replacements
and unsupported transaction metadata. Version locks stay in force. The fixed
commands disable weak dependencies and obsoletes, keep repository GPG checks,
and replay the checked transaction with DNF's installed-state checks intact.
They never use `--nogpgcheck`, `--allowerasing` or replay's ignore options.
Local package authenticity comes from the same independently verified GitHub
release/digest boundary as APT; stricter administrator DNF policy can still
refuse installation.

The validator accepts both original repository IDs and DNF's
`@stored_transaction(repo_id)` format, while keeping `@System` replacements
and the exact local LG Buddy package separate. Because newer DNF treats
stored repository RPMs as local packages, the helper independently requires
valid signatures and digests on every dependency RPM with `rpmkeys`, against
the system's trusted RPM keyring, before reporting mutation or replaying.
Missing trusted keys refuse the update; administrators must establish that
repository's trust through their normal DNF workflow before retrying.

Known Fedora 43 limitation: DNF 5.2.18 can fail replay when an update adds a
repository dependency, even with its RPM already stored. This also reproduces
with plain DNF and is tracked in [#334](https://github.com/Staphylococcus/LG_Buddy/issues/334),
a stable-readiness blocker under [#136](https://github.com/Staphylococcus/LG_Buddy/issues/136).
The current version-only beta-to-beta qualification retains the baseline's
runtime dependency set and is reported separately.
The [upstream-fix validation record](dnf-replay-validation.md) covers newer
DNF and the separate Fedora 43 compatibility assessment; this slice does not
rewrite older DNF plans or change the supported Fedora baseline.

Afterward the updater verifies the exact RPM database version, ownership and
both installed binary identities before relaunching GTK as the invoking user.
A newer release still needs publication as an RPM asset; no package repository
is installed or inferred from the update channel.

## Lifecycle checks

CI builds the native package twice and runs the real DNF/RPM lifecycle in a
disposable Fedora 43 container. The lifecycle deliberately requires root,
Docker and `LG_BUDDY_DISPOSABLE_PACKAGE_TEST=1`:

```bash
LG_BUDDY_DISPOSABLE_PACKAGE_TEST=1 bash scripts/test-rpm-package.sh \
  /packages/lg-buddy-2.0.0-beta.3-1.x86_64.rpm \
  /packages/lg-buddy-2.0.0-beta.3-2.x86_64.rpm
```

Checks cover conflicts, install, local packaging-release upgrade, reinstall,
native metadata/ownership/checksums/modes, binary identities, runtime linkage,
installed GTK launch and online/offline removal with retained user data.
An additional build injects a real optional ELF library and verifies that it
adds no private-library dependencies/provides while GTK dependencies remain.
Root package operations preserve complete current and stale profiles. The
shared installed startup regression checks root/foreground refusal, then
automatic conversion as the stale config's non-root owner without a desktop,
session bus or TV, retaining other preferences and credential bytes/access
metadata. The converted profile remains the removal retention baseline.

Running-manager commands are recorded fakes; offline enablement-link removal
uses real `systemctl`. These checks do not qualify real desktop activation,
polkit prompts or reboot. The native updater smoke additionally covers root
digest refusal, version-lock refusal, declined confirmation, verified newer
package installation, retained user data and normal-user GTK relaunch. Its
loopback HTTP transport and sudo authorization are test substitutes; real
desktop authentication and reboot are covered by the
[VM qualification](rpm-vm-validation.md). The RPM job is part of the existing
required `bundle-smoke-test` aggregate check.

A separate Fedora 44 DNF job runs the updater with `libglvnd-gles` absent from
the baseline and requires it to be installed by the update. It also exercises
the helper's stored-RPM verification with a genuine signed dependency, an
unsigned RPM, a tampered RPM and a symlink. The required smoke aggregate
includes this job. Pass `--expect-new-rpm-dependency libglvnd-gles` to the
native updater smoke to require the same dependency-changing journey locally.
