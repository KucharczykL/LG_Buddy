# Distribution support

This is the distribution and delivery policy for LG Buddy 2.0. Desktop features
also depend on the [session compatibility table](../README.md#desktop-compatibility).
Supporting a package format does not establish support for every distribution
that uses that format.

## Supported baselines and update paths

| Distribution | Installation and updates | Qualification |
| --- | --- | --- |
| Ubuntu 24.04 LTS, x86_64 | Official `.deb` with in-app APT updates, or the release bundle with its own updater | CI, native package lifecycle, public native updates and an Ubuntu desktop VM. |
| Fedora 44 or newer, x86_64, conventional writable installation | Official `.rpm` with in-app DNF5 updates, or the release bundle | Fedora 44 is the current CI/native update baseline. Newer Fedora releases must meet the same dependencies; they are not individually qualified merely by passing the version check. Fedora 44 desktop/polkit qualification remains outstanding. |
| Arch Linux, current fully updated x86_64 system | Official release bundle and bundle updater. A pacman-owned installation must be updated through its package source. | Current Arch CI installs and launches the official bundle. No official pacman package or in-app replacement of pacman-owned files. |
| Fedora 43 | Existing installations can continue to run. RPM updates use the manual path below; in-app RPM installation is refused. | Previous Fedora 43 records remain historical evidence, including its DNF replay failure. It is no longer the automatic RPM update baseline. |
| Debian, other Ubuntu releases and derivatives such as Pop!_OS | Compatibility depends on the required libraries, native package dependencies and writable system layout. A healthy LG Buddy `.deb` uses APT; the bundle uses its own updater. | Not individually qualified by the Ubuntu 24.04 baseline. A shared package manager or `ID_LIKE` value is not a support guarantee. |
| Other RPM distributions, including RHEL-family and openSUSE systems | Automatic RPM updates are refused. Bundle compatibility must be established separately. | Not qualified by Fedora testing. |
| NixOS and immutable/image-managed hosts, including Fedora Atomic desktops | The mutable-system installer/updater is unsupported; use the distribution's own packaging/integration where available. | Official immutable-host/NixOS integration is planned under [#24](https://github.com/Staphylococcus/LG_Buddy/issues/24). |

The baseline advances as distributions age out of maintenance. Use a release
still maintained by its vendor. Adding a newer baseline requires dependency,
installation and update checks; old validation records retain their original
distribution versions and do not become evidence for a newer release.

## Common requirements

- Official downloads target **x86_64 Linux**. The GTK executable requires
  **glibc 2.39, GTK 4.14 and libadwaita 1.5 or newer**. The CLI's musl build does
  not make the complete application compatible with Alpine/musl-only systems.
- Installation requires a conventional writable system layout and systemd
  integration. Desktop authorization requires polkit and a normal user session.
  The installer checks capabilities and ownership before changing files.
- Native APT/DNF updates require a matching package asset on the selected
  GitHub release. Installing a local package does not add a repository, so
  `apt upgrade` or `dnf upgrade` alone does not discover LG Buddy releases.

Release-bundle preflight checks capabilities rather than a distribution
allowlist. The stricter **Fedora 44+ rule applies to automatic RPM updates**.
The GUI and CLI check it before release downloads or administrator
authorization; the privileged helper checks it again before package mutation.
The helper reads the host's `os-release` identity and does not treat another
distribution's `ID_LIKE=fedora` as Fedora support.

## Fedora 43: update an existing RPM installation

The recommended route is to [upgrade Fedora](https://docs.fedoraproject.org/en-US/quick-docs/upgrading-fedora-new-release/)
to 44 or newer, keeping its supplied DNF5 up to date. DNF 5.2.18 on Fedora 43
can fail stored transaction replay when an application update adds a repository
dependency. Retrying the in-app update does not fix that failure.

You can instead update the RPM manually:

1. Open [LG Buddy releases](https://github.com/Staphylococcus/LG_Buddy/releases)
   and choose the release/channel you intend to install. Download its x86_64
   `.rpm` and `sha256sums.txt` from that **same release**.
2. In the download directory, verify the checksums and confirm that the RPM
   reports `OK`:

   ```sh
   sha256sum --check --ignore-missing sha256sums.txt
   ```

3. Install the verified local RPM with DNF. For example, for beta.4:

   ```sh
   sudo dnf5 install ./lg-buddy-2.0.0-beta.4-1.x86_64.rpm
   lg-buddy
   ```

   Substitute the filename of the release you actually downloaded. Review
   DNF's transaction before confirming it. This ordinary local-package path
   does not use LG Buddy's stored transaction replay.

Native upgrades retain LG Buddy settings, pairing credentials and the machine
configuration binding. Follow any dependency or signature error DNF reports.
Use the RPM path for an RPM-owned installation: the release-bundle installer
refuses to overwrite package-owned files. In-app release checks remain useful
for discovering releases, even when automatic RPM installation is unavailable.

## Early v2 RPM betas on Fedora 44+

The immutable beta.3 and beta.4 packages contain an updater that does not
understand newer DNF stored repository names. If you already installed either
on Fedora 44+, perform one manual RPM update, using the checksum/DNF procedure
above, to a later release containing the fix. Subsequent in-app RPM updates
use the corrected helper. Upgrading Fedora alone does not replace the older
LG Buddy updater.

For a later prerelease, the release canary records that known transition as
`manual-transition`, with
`automatic_upgrade_qualified=false`. It is not a beta-to-beta automatic-update
pass. A later pair of published releases containing the fixed helper is needed
to qualify that journey on Fedora 44. Stable releases and newer-baseline
failures are not eligible for the manual-transition exception.

## Evidence and developer references

- [Debian/APT guide](debian-package.md) and [Ubuntu desktop VM record](debian-vm-validation.md).
- [RPM/DNF guide](rpm-package.md), [DNF replay validation](dnf-replay-validation.md)
  and the historical [Fedora 43 desktop VM record](rpm-vm-validation.md).
- [Package ownership contract](package-ownership.md) and [release process](release-process.md).

CI/container passes cover their recorded install, runtime and update journeys.
They do not independently establish real desktop authorization, service-manager
activation, reboot or physical-TV behavior on every distribution in a family.
