# Debian package VM qualification

Recorded 2026-10-23 for [#134](https://github.com/Staphylococcus/LG_Buddy/issues/134).
This supplements the [isolated package tests](debian-package.md#validation)
with a real Ubuntu desktop, system/user service managers and polkit. No
application changes were needed during qualification.

## Environment and tested bytes

| Item | Value |
| --- | --- |
| Product source | `bd8d5e2d07639c5cf4933a58e8da0c412cb7d7e4` |
| Guest | Ubuntu 24.04.5 LTS, kernel 6.8.0-147-generic, GNOME 46 / Xorg |
| VM | Incus/KVM, 2 vCPUs, **2 GiB RAM**, 24 GiB disk, no guest swap |
| Image | `images:ubuntu/noble/desktop`, serial `20261018_07:42` |
| Accounts | Desktop/config owner UID 1000; separate headless stale-config owner UID 1001 |
| Candidate identity | Local test release `1.4.0-beta.2.ci`, full product source commit above |
| Candidate runtime / GUI | Optimized musl runtime and GNU GTK executable, built on Ubuntu 24.04 outside the VM |
| Update baseline | Same source, GNU debug transport fixtures reporting `0.0.0`, stripped and placed in the full package layout |

The test versions are local qualification identities, not published releases.
The candidate contains the reviewed APT implementation. The VM RAM allocation
was never increased; the VM was stopped after evidence capture.

SHA-256 identities:

```text
lg-buddy_1.4.0~beta.2.ci-1_amd64.deb
a7057c1dad23b7e283ed2bafc6b2ed03fdf3365c9847200097b0f148b9ec22da
lg-buddy_1.4.0~beta.2.ci-2_amd64.deb
0e520bd92e8b1e46831e8eb5ab19133ead21a8538597194e3baff1d03a7315bb
installed runtime
9394450a94d852c18f5988651af15b0b28f77e638b301dcf9145f972cb08590d
installed GTK executable
ec1c5c200c7d2590c0ada3e8c2b583b132f3574e896dfbad540d8d9232f89c2d
```

## Results

| Check | Result |
| --- | --- |
| Fresh APT installation | No user config or machine binding created; machine services remained disabled/inactive. Native version, owners, embedded identities and all payload checksums matched. |
| Shared GUI onboarding | Paired the loopback webOS fixture, then completed service setup through the actual GNOME password prompt. Machine and user services became enabled/active. |
| GTK controls | Existing AT-SPI assertions passed for brightness, volume, mute and Settings. Native Tab navigation reached the update action. Light/dark appearance and 125% text scaling retained accessible controls. |
| Root migration guard | The installed root lifecycle command refused a supported stale config and left its bytes, owner, mode and credentials unchanged. |
| Automatic owner migration | The installed screen service converted `bscpylgtv` and `swayidle` as UID 1001, without a graphical session or reachable TV. All other preferences and credential bytes/ownership/modes were retained; no acknowledgement or pairing was requested. |
| Package revision upgrade / reinstall | Real APT transactions preserved dynamic files, activation policy and the running lifecycle daemon. Vendor payload checksums still matched. |
| Real updater authorization cancellation | Dismissing the native polkit prompt left the baseline version, configuration and credentials unchanged. The GUI exposed the failure and allowed retry. |
| Verified newer-package update | GUI discovery and confirmation acquired the native package; real `pkexec` authorization and APT installed it. The privileged helper re-fetched release/tag/checksum metadata. |
| GTK handoff | The restarted GUI ran as UID 1000 and `/proc/<pid>/exe` matched the installed candidate bytes. The existing lifecycle daemon continued through the transaction. |
| Reboot | Machine and user services started automatically. After the updater reboot, the lifecycle process matched the installed candidate bytes; GUI control assertions passed and the retained token worked without pairing. |
| Removal and purge | User integration was explicitly disabled first. APT removed vendor payload and stopped/disabled machine units while retaining ten dynamic files, including both users' settings and credentials, byte-for-byte with owners/modes intact. |
| Explicit machine reset | Removed only verified, unmodified generated binding/drop-in files. An administrator-modified startup drop-in and all user data were preserved. |

## Substitutions and limits

The TV was the repository's loopback webOS fixture; no physical TV was used.
Service managers, package tools, desktop session, authentication and executable
handoff were real. There were no service-command stubs, NOPASSWD update rule or
polkit authorization bypass.

Release HTTP transport used the existing debug-only GitHub fixture. Because
`pkexec` clears caller variables, the disposable guest's PAM session added only
the loopback fixture address from a root-owned envfile; the existing common
authentication/account/session stack remained in place. This uses
[pkexec's PAM session environment handling](https://github.com/polkit-org/polkit/blob/master/src/programs/pkexec.c).
That guest-only addition was removed afterward. The optimized candidate cannot
enable the debug transport feature. Two package acquisitions covered cancelled
and approved attempts; three checksum acquisitions included root revalidation.

Public assets, actual GitHub package transport and published beta/stable update
canaries remain [#136](https://github.com/Staphylococcus/LG_Buddy/issues/136),
which also requires the [RPM delivery](https://github.com/Staphylococcus/LG_Buddy/issues/135).
This is Ubuntu/GNOME smoke qualification, not a physical-TV, other-desktop or
long-duration runtime result.

## Reproduction and evidence

Build the two executables from the product source above with the same full
commit and test version, use `scripts/build-release-bundle.sh`, then build
package revisions 1 and 2 with `scripts/build_debian_package.py`. Use a fresh
Ubuntu 24.04 desktop VM; install packages with APT and run onboarding as its
logged-in user. Run the installed `LG_Buddy_screen.service` as a second,
non-graphical config owner for the stale-config check. The existing
`scripts/test-release-gui-accessibility.py` assertions can be scoped to the
LG Buddy accessible application on a full desktop, so GNOME's own tree does
not exhaust the container observer's node budget.

The initial TV setup selected HDMI 1, which the fixture does not model. After
selecting HDMI 3 through application settings, reboot startup completed and
reused the saved token. The image's network renderer was also corrected before
the final reboot checks. These harness/image corrections did not change the
application or package recipe. A TV powered off by package removal correctly
produced a read failure until the next normal startup powered the fixture on.

Receipts, harness scripts, screenshots, package/build artifacts and the guest
evidence archive are preserved on the validation host under
`~/.local/state/lg-buddy-validation/debian-134-20261023/`. The archive contains
the guest scripts, fixtures, selected test-user data, APT/dpkg logs and service/
authorization journals. Its SHA-256 is:

```text
vm-evidence.tar.gz
03b64c37461887c0c1e3dc468e8f24ba10671a3f0cfcb50092f648dd850b152f
```
