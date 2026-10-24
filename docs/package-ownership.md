# Package ownership contract

This is the common contract for Debian and RPM delivery under
[#133](https://github.com/Staphylococcus/LG_Buddy/issues/133). The ownership,
provisioning, binding, installer and updater prework is implemented. Debian
package creation and its APT adapter have isolated coverage and
[Ubuntu VM qualification](debian-vm-validation.md) under
[#134](https://github.com/Staphylococcus/LG_Buddy/issues/134).
RPM package creation and its DNF5 adapter have isolated coverage and
[Fedora VM qualification](rpm-vm-validation.md) under
[#135](https://github.com/Staphylococcus/LG_Buddy/issues/135).
The [release train](release-process.md#native-release-assets) integrates public
package assets and update canaries under
[#136](https://github.com/Staphylococcus/LG_Buddy/issues/136); production
qualification requires the recorded published update journeys.

## Ownership invariant

LG Buddy never directly mutates package-owned base assets. In package mode,
application setup writes persistent dynamic configuration only in user storage,
`/etc`, or an explicitly defined writable `/var/lib` location. Vendor base
assets are verify-only for the application; their owning package manager
installs, replaces and removes them.

The one vendor-path exception is a **host-integration artifact**: an individual
KWin plugin may be installed at its loader-required location when the artifact
is unowned and the destination is mutable. This never authorizes replacing a
package-owned plugin. See [KWin artifacts](#kwin-artifacts).

The release-bundle installer remains the payload installer for recognized,
unowned bundle layouts, including their existing `/usr` payload paths. It is
not an alternative writer for a package-managed installation. The
[clobber guard](#bundle-clobber-guard) enforces that boundary.

### Ownership observation

The shared `PackageDatabase` answers a fallible path-level question:

```rust
owner_of(path) -> Result<PathOwnership, PackageDatabaseError>
// PathOwnership: Unowned | Owned(PackageOwner) | Conflicting(Vec<PackageOwner>)
```

`PackageOwner` carries the manager family as data: `Dpkg`, `Rpm`, or `Pacman`.
The database aggregates host ownership observations, not `/etc/os-release`,
an LG Buddy receipt, or an assumption that a missing file is unowned. Absent
database executables may be ignored when another database answers; no available
database, a present but failing probe, or malformed output is an error.
Production probes use fixed executable paths and remove environment overrides
that could redirect the package database.

Every mutation gate refuses `Owned`, `Conflicting`, and `Err`; only a verified
`Unowned` answer can permit an application write. Ownership of an existing
shared container directory does not prevent traversal, but the application
must not change that directory's ownership or mode. Newly created paths and
individual files still need their own checks.

The installation-level probe asks about `/usr/bin/lg-buddy`:

| Observation | Installation ownership |
| --- | --- |
| One package owns the executable | `Package { family }` |
| Executable unowned and installed layout passes bundle preflight | `Bundle` |
| Unowned but incompatible layout, conflicting owners, or probe error | `Unknown` |

An installation-level result does not replace destination-level checks.
Future host families extend the ownership data and their supported delivery
adapter; they do not bypass the common mutation rules.

### Filesystem authority

A permitted root is not unconditional write permission. Check canonical paths,
symlink redirection, expected file types, ownership and the destination's
filesystem before mutation. For a missing destination, inspect the existing
ancestors and applicable mount. Read-only means host filesystem/mount facts
(`statvfs` and mount observations), not `test -w` as the invoking user. Refuse
unsafe mount-point replacement/removal and failures to obtain these facts.

NixOS and OSTree retain their existing managed-host boundary. Application
repair does not introduce an imperative deployment mechanism there; external
configuration/build/activation or the supported image mechanism owns repair,
followed by read-only verification. A writable KWin overlay is outside v2.

## Paths and writers

Both package families use the following application layout. Paths containing
`$XDG_CONFIG_HOME` use its absolute value, normally `~/.config`; existing custom
config locations and their adjacent credentials stay in place.

| Asset / path | Installed or maintained by | Application authority in package mode |
| --- | --- | --- |
| `/usr/bin/lg-buddy`, `/usr/bin/lg-buddy-gui` | Package manager | Execute and verify; never replace directly |
| Fixed helpers under `/usr/lib/lg-buddy/`, including `setup-services` and `setup/systemd/` templates | Package manager | Execute/read; template damage requires package repair |
| `/usr/lib/lg-buddy/kwin/` shipped sources, metadata and prebuilts | Package manager | Read/verify as provisioning inputs |
| `/usr/lib/systemd/system/{LG_Buddy,LG_Buddy_lifecycle}.service` | Package manager | Verify base units; authorized setup manages activation |
| `/usr/lib/systemd/user/LG_Buddy_screen.service`, `LG_Buddy_update_check.service`, `LG_Buddy_update_check.timer` | Package manager | Verify bases; the user's setup manages activation |
| `/usr/lib/tmpfiles.d/lg_buddy.conf` | Package manager | Verify; authorized setup invokes tmpfiles |
| `/usr/lib/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle` | Package manager | Verify; invoke the installed machine-context entrypoint |
| `/usr/share/applications/io.github.staphylococcus.LGBuddy.desktop`, `/usr/share/icons/hicolor/scalable/apps/io.github.staphylococcus.LGBuddy.svg`, `/usr/share/polkit-1/actions/io.github.staphylococcus.LGBuddy.setup.policy` | Package manager | Read/use; no per-user data in these files |
| Documentation under `/usr/share/doc/lg-buddy/` and the family's license location | Package manager | Read-only; recipe owns native documentation/license metadata |
| `/etc/systemd/system/{LG_Buddy,LG_Buddy_lifecycle}.service.d/config.conf` | Authorized application setup | Write unowned dynamic `LG_BUDDY_CONFIG` bindings |
| `$XDG_CONFIG_HOME/systemd/user/{LG_Buddy_screen,LG_Buddy_update_check}.service.d/config.conf` | User's application setup | Write that user's unowned dynamic bindings |
| `/etc/lg-buddy/config-path` | Authorized application setup / legacy binding migration | Root-owned machine selection; never a package-owned config file |
| `$XDG_CONFIG_HOME/lg-buddy/config-path` | User's explicit setup / legacy install transition | User-owned selection record for subsequent user-context lookup |
| Selected `config.env` and adjacent `tvs/primary/access-token.json` | Config owner through application settings, migration and pairing | Preserve location and owner; never package payload or scriptlet input |
| User cache/state/runtime locations, including KWin cache and receipts | User's application / provisioner | Maintain only that user's state |
| `/run/lg_buddy` | Existing tmpfiles rule and runtime | Volatile runtime state with the existing access policy |
| `/run/lg-buddy-package-updates/` | Privileged APT adapter | Root-private temporary package copy and acquisition lock; never user configuration |
| `/var/lib` | No new persistent application store is introduced by this contract | Any future machine store needs an explicit layout and owner |

Runtime files under `/run`, user runtime directories and temporary/cache files
are not vendor base assets or new persistent system configuration. Packages
must not claim the dynamic binding files, even as database-only/ghost files:
that would make setup correctly refuse to update them.

Base units and templates are root-owned mode `0644`; executable payloads and
the dispatcher hook are mode `0755`. Dynamic binding files are regular mode
`0644` files owned by root or the relevant user. Binding directories must have
the expected owner and no group/other write permission; redirection is refused.
Credentials retain their existing private storage protections.

The provisioner verifies vendor assets against the shipped templates, owner,
type and mode. Missing/modified assets and local base units that shadow them
block package-mode repair. Repair belongs to the package manager or explicit
administrator remediation, not a fallback copy into `/etc`. Bundles retain
their established local base-unit layout; packages must not silently adopt it.

## First run and activation

Package installation is noninteractive payload installation. It does not
enable/start services, choose a desktop user, bind a configuration, pair a TV,
launch UI, or convert user settings. Generated maintainer scripts must obey
this policy too. A machine-scoped daemon reload does not constitute activation.

The installed application's existing [onboarding flow](onboarding.md) owns
first-run configuration, pairing and applicable service/integration setup.
GTK renders domain-owned states and returns semantic intents; packaging does
not add a second setup or migration workflow. Required services are verified
and enabled through this flow; the update timer follows `updates.auto_check`.
Saved behavior settings are preserved rather than rewritten to hide an unmet
setup requirement. Cancellation or failed verification leaves setup pending.

In package mode, repair verifies vendor bases and writes only the dynamic
bindings above. The privileged fixed-scope service helper reobserves ownership;
the frontend cannot select an unowned repair mode. Stop/rebind/reload/restart
ordering and subsequent readiness checks recover interrupted repair without
claiming a matching file proves an already-running process has the right config.

## User configuration and machine binding

User-context GUI, CLI and user-session operations resolve configuration in this
order: explicit `LG_BUDDY_CONFIG`, the validated user selection record, then
the user's normal XDG/home default. The existing sudo-user discovery policy
remains user-context discovery. It does not turn into machine lookup because
the effective UID changed. An absent user record permits default discovery;
an invalid existing record reports an error rather than silently selecting
another configuration.

Machine integrations use their explicit `LG_BUDDY_CONFIG` service drop-in.
Machine entrypoints requiring discovery, such as `nm-pre-down`, instead use
the validated root-owned `/etc/lg-buddy/config-path` when no explicit path is
provided. That record contains one absolute config path, not settings or
credentials. Missing/invalid binding does not fall back to a user's home or the
legacy pointer; `nm-pre-down` reports a binding error. Entry-point context is
explicit, not an effective-UID, active-session or seat heuristic.

One selected configuration controls machine-wide integrations. The pointer is
not a GUI/CLI default for all desktop users. Authorized setup may create or
rebind it while updating the corresponding system-service bindings. Ordinary
reads/settings changes, package installation and payload updates do not rebind
it. Multiple machine selections and active-user arbitration are outside scope.

### Existing-install transition

`/usr/lib/lg-buddy/config-path` is legacy installation/migration input only.
It is never a normal runtime fallback. Updater upgrades and manual installer
reruns use the same binding transition:

1. Validate the selected configuration, user selection and installed binding.
   A valid new machine binding takes precedence over stale legacy input.
   Invalid new records or conflicting explicit selections refuse the transition.
2. Preserve the config and credential files at their existing locations. Record
   a legacy non-default selection in the user's own selection record so later
   ordinary GUI/CLI launches can still find it.
3. Atomically publish the root-owned `/etc` binding before restarting services.
   Ownership, writable-filesystem and path-redirection checks still apply.
4. Repeated/interrupted execution must preserve a valid new binding and retained
   user data; a partial service change remains incomplete until repaired and
   verified. Do not claim the entire multi-file installation is atomic.

This moves selection authority, not user settings. Payload installation leaves
supported stale 1.x contents for the installed application's existing startup
conversion under [#137](https://github.com/Staphylococcus/LG_Buddy/issues/137).
The [GUI and user screen daemon's startup boundary](../crates/lg-buddy/src/startup.rs)
converts as the config's non-root owner before config-dependent workers start. Conversion is local,
preserves preferences/comments/credentials, and does not require acknowledgement,
a migration screen, TV access or desktop readiness. The root lifecycle daemon
does not convert; its current-config guard fails until the owner daemon publishes
the converted file, after which its existing restart policy can recover.
Foreground TV operations likewise cannot use unconverted settings. Read-only
diagnostic/protocol paths retain their no-conversion boundary. Package scripts
must not invoke conversion or inspect per-user migration state. Pairing and
runtime/setup readiness remain separate checks after conversion.

## Bundle clobber guard

The bundle installer supplies the complete planned system mutation set to the
candidate's private `installer-preflight` before dependency installation,
temporary installation files, config/selection writes or service changes.
The plan includes missing destinations, recursive removals, copied payload
members and the exact legacy enablement links to remove. Any owned/conflicting
destination, failed ownership query or unsafe filesystem observation refuses
before mutation and leaves the installation/user state unchanged.

Candidate compatibility checks reject structurally invalid upgrades before
expensive ownership walks. The authorized graphical upgrade helper rechecks
the mutation guard before system writes. A later I/O failure after mutations
begin follows the existing partial-upgrade recovery contract; preflight is
not a transaction or permission to overwrite files installed by another owner.

## KWin artifacts

The shipped `/usr/lib/lg-buddy/kwin` payload stays verify-only during setup.
A compatible prebuilt or verified local build is copied through the same
guard into `/usr/lib*/qt6/plugins/kwin/plugins`, the loader-required location.
Installation and removal use `check_host_artifact` plus the provisioner's fixed
loader-root, user/content-addressed filename and integrity checks. The target
file must be unowned and mutable; existing vendor-owned container directories
are traversed without chmod/chown. Ownership errors/conflicts, redirected
ancestors, read-only filesystems and managed-host markers refuse mutation.

Local builds use `$XDG_CACHE_HOME/lg-buddy/kwin`; receipts use
`$XDG_STATE_HOME/lg-buddy/kwin`. No KWin installation, compiler, development
package or download is forced on every package user. Compilation is an explicit
application integration operation with separate dependency consent, never a
package scriptlet. Existing compatible host-provided bridges can be verified.
Absent/rejected bridges preserve operation with available sources, without
pretending an applicable incomplete setup requirement passed. Compatibility,
loader proof and fallback details remain in [KWin integration](kwin-integration.md).

## Update ownership and release identity

The implemented updater checks installation ownership before release discovery
and again after acquisition, before candidate/installer execution. Bundles and
healthy LG Buddy Debian and RPM packages share the install action. Other owners
receive ownership guidance; uncertainty and changed ownership invalidate a stale
offer.

| Ownership | Implemented behavior | Remaining delivery work |
| --- | --- | --- |
| `Bundle` | Verified release-bundle installation | Preserve this path for existing bundles and Arch |
| `Package { Dpkg }` | Verified local `.deb` acquisition and APT for the healthy `lg-buddy` package | #136 published update qualification |
| `Package { Rpm }` | Verified local `.rpm` acquisition and DNF5 for the healthy `lg-buddy` package | #136 published update qualification |
| `Package { Pacman }` | Refuse self-install; show pacman guidance | Externally managed; no pacman delivery in v2 |
| `Unknown` | Refuse automatic installation | Remains refused, including conflicting/error observations |

APT/DNF delivery reuses the saved update channel, release discovery,
explicit confirmation, verification and installed-app relaunch flow. Package
managers own dependency resolution and system mutations. A downloadable local
package does not establish an APT/DNF repository or promise that a normal
repository upgrade discovers LG Buddy releases.

The [Debian](debian-package.md#integrated-updates) and
[RPM](rpm-package.md#updates) adapters additionally recheck
the selected delivery kind after acquisition. The installed privileged runtime
revalidates the official release digest, stages only matching bytes under root
control and invokes APT or DNF5 with fixed arguments and a clean environment.
Native package state, ownership and both binaries are verified after the transaction.
The native package container smoke tests substitute HTTP transport and
authentication; desktop service managers, polkit and reboot also have separate
[Ubuntu](debian-vm-validation.md) and [Fedora](rpm-vm-validation.md) VM
qualification.

### Common identity tuple

Both binaries expose the same logical `(version, channel, commit)` identity;
release tags, target architecture and payload checksums are verified alongside
it. Official builds inject `LG_BUDDY_RELEASE_VERSION` and
`LG_BUDDY_BUILD_COMMIT`; an ordinary development build may report `dev` and
`unknown`, which is not sufficient identity for a verified release update.

For example, an illustrative prerelease build has this identity:

```text
version: 1.10.0-beta.1
channel: prerelease
commit: 0123456789abcdef0123456789abcdef01234567
```

The package version is a family serialization of the expected release version,
not a serialization of the whole tuple. Debian and RPM prereleases use `~`
ordering: `1.10.0~beta.1` precedes `1.10.0` under
[Debian's version rules](https://www.debian.org/doc/debian-policy/ch-controlfields.html#version)
and [RPM's version rules](https://rpm.org/docs/latest/manual/spec.html#version).
Illustrative package strings are Debian `1.10.0~beta.1` and RPM
`1.10.0~beta.1-1` (Version-Release). #134/#135 own their exact revision/release
encoding and ordering tests; no common package-version format carrying a Git
SHA is introduced.

After a package update, verification must establish all of:

1. The manager-reported installed package version equals that family's
   serialization of the expected release version, including its defined
   revision/release component.
2. Installed runtime and GTK binary identities match the verified expected
   version/channel/commit, with the expected architecture and usable GUI ABI.
3. The expected package manager owns the installed executable.

Install the verified payload first, verify the installed result, then relaunch
the installed foreground GUI when the invoking session supports it. Application
startup owns any config conversion; package mutation does not. Preserve the
existing distinction between failed installation, partial mutation and a
successful installation whose relaunch needs retry.

## Runtime dependencies and qualification baselines

The native Rust runtime needs no Python environment; the GTK executable uses
host shared libraries. Its enforced runtime minimum is **GTK 4.14 and
libadwaita 1.5**, as checked by `lg-buddy-gui --check-runtime` without opening a
display.

| Delivery / qualification baseline | Required GUI packages |
| --- | --- |
| Debian family: Ubuntu 24.04 (#134) | `libgtk-4-1` >= 4.14, `libadwaita-1-0` >= 1.5 |
| RPM family: Fedora 43 (#135) | `gtk4` >= 4.14, `libadwaita` >= 1.5 |
| Release bundle: Ubuntu 24.04, Fedora 43, Arch current | Same minimums; Arch package names are `gtk4`, `libadwaita` |

Package format/family is not a claim that every Debian/RPM distribution is
supported. New baselines require dependency and launch/linkage qualification.
Recipes must declare the actual runtime library requirements of both payloads,
including generated ELF dependencies and these explicit GUI minimums. Build
and test dependencies are separate. Python, bscpylgtv, Zenity and swayidle are
not runtime dependencies of a native installation; optional integration/build
dependencies remain conditional on the relevant application operation.

Release bundles keep the musl headless executable and GNU GTK companion. Their
existing linkage check requires a static headless runtime, resolved GUI shared
libraries and GUI GLIBC requirements no newer than 2.39. Package recipes must
qualify their own actual binary linkage and metadata on the named baseline;
using a different package build does not relax the GUI minimums. Preserve
standard GTK controls, accessibility, keyboard, scaling and theme behavior.

## Lifecycle, retention and transition

These are package-recipe acceptance requirements, alongside the implemented
setup/binding guards above:

| Operation | Required behavior |
| --- | --- |
| Fresh install | Install package-owned payload noninteractively; no service activation or per-user work |
| Upgrade / reinstall | Replace payload through its manager; retain selected config, credentials, user selection, machine binding and saved update channel; never silently rebind to the invoking user |
| Service handling on upgrade | Preserve prior activation policy; machine-scoped reload/restart must not enable a previously disabled installation, invoke user setup, or perform scriptlet-driven config conversion |
| Ordinary removal | Stop/disable the package's system services and remove its owned payload through the manager; retain dynamic configuration and all user settings/credentials/state |
| Explicit purge / reset | A separately documented, explicit machine reset may remove only identified LG Buddy-generated machine bindings/drop-ins safely; preserve modified/foreign files for administrator resolution. It never follows the config pointer into a user's data |
| User-data reset | A separate explicit operation by that user; never implicit in install, upgrade, removal or machine purge |

#134/#135 must document their concrete native removal/reset procedure and
validate it; this contract does not introduce a generic purge command. Package
scripts do not scan homes, invoke user UI or use root to mutate user settings.
Administrator-modified vendor files require package-native repair and conflict
handling. Application setup refuses them rather than silently restoring them.

There is no automatic bundle-to-package adoption. Overlapping payloads or local
base units must be detected and resolved before package installation replaces
them. A manual transition records the selected config, preserves its settings
and credentials, stops/removes the old bundle payload through that installation's
maintenance path, installs the package, and runs explicit setup as the config
owner. Package setup must not erase conflicting local units to make itself pass.
Side-by-side ownership of the same paths and automatic cross-family conversion
are unsupported. A package installation is removed/repaired through its package
manager, not the release-bundle installer or uninstaller.

## Implementation and acceptance map

| Prework | Contract sections | Implementation and existing evidence |
| --- | --- | --- |
| #301: ownership probe | Ownership observation; update ownership | [package database and module tests](../crates/lg-buddy/src/upgrade_preflight/package_ownership.rs) |
| #302: package provisioning | Paths and writers; first run and activation | [provisioner](../crates/lg-buddy/src/setup/provision.rs), [package fixtures](../crates/lg-buddy/src/setup/provision/tests.rs), [privileged helper smoke](../scripts/test-service-setup.sh) |
| #303: config/binding separation | User configuration and machine binding; existing-install transition | [lookup policy](../crates/lg-buddy/src/config.rs), [binding publisher/tests](../crates/lg-buddy/src/config/binding.rs), [entrypoint fixtures](../crates/lg-buddy/tests/config_binding.rs), [installed upgrade evidence](../scripts/test-cross-version-upgrade.sh) |
| #304: clobber guard | Filesystem authority; bundle clobber guard | [guard and module fixtures](../crates/lg-buddy/src/upgrade_preflight/installer.rs), [installer](../install.sh), [bundle lifecycle smoke](../scripts/test-release-bundle.sh) |
| #305: updater refusal | Update ownership and release identity | [installer orchestration/tests](../crates/lg-buddy/src/update_install.rs), [stale GUI offer handling/tests](../crates/lg-buddy/src/update_flow.rs) |
| #307: KWin ownership | KWin artifacts | [native privileged operations](../crates/lg-buddy/src/setup/kwin/native.rs), [privileged entrypoint fixtures](../crates/lg-buddy/tests/kwin_ownership.rs), [native helper smoke](../scripts/test-kwin-setup.sh) |
| #306: common contract | This document | Shared reference for Debian/RPM recipes and future immutable-host work |

The application prework is testable without producing either package, using
the existing injected package database, `system_root`/`LG_BUDDY_INSTALL_ROOT`
fixtures and private filesystem namespaces. The normal Rust suite covers these
contracts; [development validation](development.md#validation) names the commands.

#134/#135 must additionally prove real package metadata/ownership, ordering,
clean install, verified local-package update, reinstall, interrupted/refused
operations, remove/reset retention, automatic stale-config startup conversion,
and GUI launch. Isolated package tests are supplemented by install, first run,
upgrade, reboot and removal on actual Ubuntu/Fedora VMs. #136 adds exact-commit
asset publication/checksum/digest verification and public package update
canaries, while retaining bundle/Arch and historical bundle-upgrade coverage.
Those delivery/real-host results must be recorded separately from prework
fixture success. The [Ubuntu VM record](debian-vm-validation.md) and
[Fedora VM record](rpm-vm-validation.md) document the Debian and RPM results.
Public package update qualification is recorded separately under #136.
