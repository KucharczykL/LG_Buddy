# RPM package VM qualification

Current distribution/update policy is documented in
[distribution-support.md](distribution-support.md). This historical record
retains its original Fedora 43 environment; it does not qualify the new Fedora
44 automatic-update baseline.

Recorded 2026-10-24 for [#135](https://github.com/Staphylococcus/LG_Buddy/issues/135).
This supplements the [isolated RPM tests](rpm-package.md#lifecycle-checks) with
real Fedora desktop, package, service-manager and administrator authorization
paths.

## Environment and tested bytes

| Item | Value |
| --- | --- |
| Product source | `73b5abdc1a1695275ac4fc74331ff1420b7ca0f5` |
| Guest | Fedora 43, kernel `7.2.11-100.fc43.x86_64` |
| Desktop | GNOME Shell 49.10 / Mutter 49.8, Wayland |
| GTK / libadwaita | 4.20.4 / 1.8.8 |
| Package tools | RPM 6.0.2, DNF5 5.2.18.0 |
| VM | Incus/KVM, 2 vCPUs, **2 GiB RAM**, 24 GiB disk, 1 GiB disk swap |
| Image | `images:fedora/43/cloud`, serial `20261021_20:33` |
| Accounts | Desktop/config owner UID 1001; headless stale-config owner UID 1002 |
| Candidate identity | Local test release `1.4.0-beta.2.ci`, product source commit above |
| Candidate binaries | Optimized musl runtime and GNU GTK executable, built on Ubuntu 24.04 outside the VM |
| Update baseline | Same source, GNU debug transport fixtures reporting `0.0.0`, placed in the full RPM layout |

These are local qualification identities, not published releases. RAM was never
increased. The cloud image expanded its root filesystem to the allocated disk
before desktop installation. GNOME and the package journeys used real managers;
the VM was stopped after evidence capture.

SHA-256 identities:

```text
lg-buddy-1.4.0~beta.2.ci-1.x86_64.rpm
138f7c9c8593da9d421ec8bd65601599700f928e0955664ff9be49029472d5fb
lg-buddy-1.4.0~beta.2.ci-2.x86_64.rpm
cc40fa0dc96aff108cd1aafbe3a80538fd6111f97657624401b9d4ef9d8ad6d2
installed runtime
606603ca6b55f7c49a9d66d9b838dc554ae70eedaa01e0afb3cd29616af4cfb4
installed GTK executable
03c22aacb67801992ea10edcf6c72cefb13de9531f90af3c404593f634371ba6
```

The package timestamp is the product commit's Unix timestamp, `1792795091`.
Repeated builds of revision 2 produced identical bytes. The RPM recipe also
explicitly retains the `rpm` tools as a runtime dependency for verification and
updates, alongside its scriptlet dependency.

## Results

| Check | Result |
| --- | --- |
| Fresh DNF installation | Native version, owners, both embedded identities and all payload checksums matched. No user config or machine binding was created; machine services remained disabled/inactive. |
| Shared normal-user onboarding | Paired the loopback webOS fixture with HDMI 3, then authorized background-service setup through the real GNOME password prompt. Machine and user integration became enabled and the monitors became active. |
| GTK controls | Existing AT-SPI assertions passed. Brightness, volume and mute actions changed the fixture's observed state to 37, 42 and muted. |
| Root migration guard | The installed root lifecycle command refused the supported stale config without changing its bytes, ownership, mode or credentials. |
| Automatic owner migration | The installed screen service converted `bscpylgtv` and `swayidle` as UID 1002 without a graphical session or reachable TV. Other preferences and credential bytes/access metadata were retained. |
| Package revision upgrade / reinstall | Real DNF transactions preserved all ten tracked dynamic files and machine activation policy. The running lifecycle PID was retained; RPM payload verification passed. |
| Update authorization cancellation | Cancelling the native polkit prompt left the baseline `0.0.0-1` and all ten tracked files unchanged. The GUI reported the failure and offered another attempt. |
| Verified newer-package update | GUI discovery and confirmation selected the RPM through the saved prerelease channel. Real `pkexec` authentication and DNF installed it; the root helper independently re-fetched release/tag/checksum metadata. |
| GTK handoff | The replacement GUI ran as UID 1001, and `/proc/<pid>/exe` matched the installed candidate hash. Native database version, owners, both identities and checksums matched; the lifecycle PID and tracked files were retained. |
| Updated reboot | Machine and user services started automatically. The lifecycle process matched the installed runtime hash, GTK controls were ready, and the stored token worked without another pairing prompt. All ten files retained their bytes/owners/modes. |
| RPM removal | User integration was explicitly disabled first. DNF removed vendor payload and stopped/disabled machine units while retaining all ten tracked files, including both users' settings and credentials. |
| Explicit machine reset | Removed only the verified generated machine pointer and lifecycle drop-in. Preserved the administrator-modified startup drop-in and all user data. |

## Substitutions and limits

The TV was the repository's loopback webOS fixture, started independently at
boot; no physical TV was used. Package tools, service managers, Wayland desktop,
authentication and executable handoff were real. There were no service-command
stubs or passwordless update authorization rules.

Release HTTP transport used the existing debug-only GitHub fixture. The guest's
temporary `/etc/pam.d/polkit-1` preserved Fedora's vendor authentication,
account and session includes, adding only the loopback fixture address through
`pam_env` for the root helper's debug transport. This override was removed and
the original vendor fallback verified before the final reboot. Two package
acquisitions covered cancelled and approved attempts; three checksum acquisitions
included independent root revalidation. The optimized candidate cannot enable
this transport substitution.

The desktop observer scoped its tree to LG Buddy and deduplicated identical
remote objects exposed through multiple GNOME accessibility paths before
running the existing assertions. A concurrent test polling query encountered
[RPM 6's keyring-lock warning](https://github.com/rpm-software-management/rpm/issues/4110);
package-script ownership queries use its read-only-query workaround, without
disabling incoming package trust checks.

An earlier default-image guest was abandoned before LG Buddy installation after
PID 1 crashed during GNOME preparation. Its diagnostics were retained; no result
from it is used as qualification evidence. The cloud-image journey above passed.

Public assets, real GitHub package transport and published beta/stable canaries
remain [#136](https://github.com/Staphylococcus/LG_Buddy/issues/136). This is a
Fedora/GNOME smoke result, not other-desktop or long-duration qualification.

## Reproduction and evidence

Build both binaries from the product source above using the stated test version
and commit, assemble the release bundle, then build RPM revisions 1 and 2 with
`scripts/build_rpm_package.py`. Use a fresh Fedora 43 cloud VM with GNOME,
ordinary user login and polkit. Follow the install, lifecycle and removal
procedures in the RPM guide. For the update journey, use the debug-only full-layout
baseline and deterministic GitHub fixture with the optimized candidate RPM.

Build/package artifacts, receipts, harnesses, screenshots, authorization/service
journals, DNF history and guest evidence are preserved on the validation host at
`~/.local/state/lg-buddy-validation/rpm-135-20261024/`.
The guest evidence archive's SHA-256 is:

```text
vm-evidence.tar.gz
5509f25a7025c1707048def73e0b76da4d3382ce93a94ce9fcc85a4b4d0017e2
```
