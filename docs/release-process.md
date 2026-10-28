# Release Process

LG Buddy uses three persistent branches as source channels:

- `main` points to the current stable release.
- `prerelease` is equal to `main` or ahead of it and points to the current
  prerelease when ahead.
- `dev` is the ordinary integration branch and may contain unreleased work.

The intended ancestry is `main <= prerelease <= dev`. Ordinary changes merge
into `dev`. An official release requires a promotion PR whose head is the
same-repository `dev` branch and whose base is `main` or `prerelease`.

The release workflow builds and verifies Debian and RPM packages from the same
release bundle. Local build and test commands are in the
[development guide](development.md#native-packages).

## Promotion contract

The promotion PR is the code and release-identity review surface. Required checks
gate its merge, and merging the PR authorizes preparation of a verified release
draft. The merged target-branch commit is the release commit. Publishing that
draft is a separate explicit approval after its release notes and assets have
been reviewed.

The release App is not a substitute for merging. After the merged commit has
passed the release build and smoke test, the App performs protected tag and
draft-release writes and keeps the persistent streams aligned. A prerelease merge
advances `dev` to the merged prerelease commit. A stable merge advances both
`prerelease` and `dev` to the merged stable commit.

Required promotion checks prove that:

- the PR is `dev -> main` or `dev -> prerelease`
- `Cargo.toml` and `Cargo.lock` declare the same `lg-buddy` version
- a stable target has a stable SemVer and a prerelease target has a prerelease
  SemVer
- the release version has an unambiguous Debian/RPM representation; unsupported
  prerelease forms refuse promotion before merge
- the version advances both existing release-channel heads
- the persistent branches have not moved or diverged before merge
- the derived `v<crate-version>` tag is absent before merge
- normal CI and the release-bundle smoke test pass; the bundle smoke includes a
  pinned cross-version upgrade from `v1.4.0-beta.2`

Requiring every candidate to advance both release-channel heads keeps release
publication globally monotonic. The newest published release is therefore also
the highest semantic version and prerelease-channel clients do not scan release
history to determine ordering.

The tag, binary, archive, and GitHub release all use the Cargo package version.
There is no separate version input.

## Creating a release

1. Prepare the exact release version in `Cargo.toml` and `Cargo.lock` on `dev`.
2. Open a PR from `dev` to `prerelease` or `main`.
3. Wait for `verify`, `bundle-smoke-test`, and `validate-promotion` to pass.
4. Merge the promotion PR. This authorizes preparation of the release draft.
5. The resulting push starts the serialized release workflow, which builds and
   smoke-tests the merged release commit without write credentials, including
   an archive-driven upgrade from the pinned public baseline. Ubuntu 24.04 and
   Fedora 44 build the Debian and RPM packages from that same verified bundle.
   Both run the real package lifecycle/startup smoke on those release bytes;
   revision 2 is a private upgrade fixture and is not published. Assembly
   verifies every format's metadata and embedded binary identities
   before generating the complete checksum file and final workflow artifact.
6. The staging job obtains a short-lived token from the dedicated repository-only
   GitHub App, aligns the remaining release streams, creates or resumes the exact
   release draft, uploads any missing assets, and verifies the complete asset set.
7. Fill in the draft's release-specific notes and review its title, classification,
   tag, bundle, Debian package, RPM and checksum file. Then approve the waiting
   `release-publication` environment.
   The approval does not accept an independent version, tag, commit, artifact, or
   workflow-run identifier.
8. The publication job downloads the same workflow artifact again, revalidates the
   merged refs, tag, checksums, bundle/native identities, draft classification, complete remote
   asset set, and non-placeholder notes, then makes only that draft public. It
   preserves the reviewed title and notes. Repository release immutability then
   locks the tag and assets.
9. A published prerelease then runs `v1.4.0-beta.2` through the real production
   `lg-buddy updates install` path. The newly installed candidate performs a
   cold-cache production update check, and the canary records sanitized GitHub
   response evidence for the deterministic mock. This is supplemental
   post-publication evidence, not a prerequisite for stable promotion.
10. Every published release also runs the Ubuntu/Fedora native package canaries
    and the pinned v1.8 bundle/startup migration check described below.

Promotion through `prerelease` is optional. A direct `dev -> main` promotion
runs the same required PR validation and post-merge release build as a
`dev -> prerelease` promotion. During stable draft staging, the release App
fast-forwards both `prerelease` and `dev` to the reviewed stable commit.

Replace the draft's placeholder with concise, release-specific notes before
approving publication. The workflow does not impose a notes template, but it
refuses to publish empty, whitespace-only, or unchanged placeholder notes.

Do not push version tags manually. Protected `v*` tags and stream-alignment
writes permit bypass only to the dedicated release App. A failed post-merge
release run can be rerun safely: an incomplete draft remains private and is
resumed only when its tag, classification, and existing assets match the merged
release commit. If publication was attempted before the notes were ready, update
the draft, rerun the failed jobs, and approve the environment again. The staged
workflow artifact is retained for 35 days, leaving a margin beyond GitHub's
30-day maximum wait for an environment approval; after that, rerun the whole
workflow to rebuild it. A published release is accepted only when its expected
asset set is complete and byte-for-byte identical.

Repository release immutability must remain enabled. GitHub applies it only when
a draft is published, so the publisher uploads and verifies every expected asset
before making the release visible. Each stored asset must report the expected
name, uploaded state, byte size, and server-computed SHA-256 digest. Unexpected
draft assets block publication; published releases are verification-only and are
never repaired in place.

## What the release workflow validates

The workflow:

1. Validates the merged target-branch commit, Cargo version, and tag state.
2. Builds a static `x86_64-unknown-linux-musl` binary with exact version and
   commit identity.
3. Generates a versioned identity manifest and packages the release bundle.
4. Validates the manifest and installs the bundle in an isolated smoke-test root.
5. Verifies the built and installed binary's exact version, channel, and commit.
6. Upgrades a pinned real previous archive to the candidate and verifies
   preserved user state plus replaced owned integration files.
7. Builds both native packages, verifies their identities, then generates and
   verifies `sha256sums.txt` covering the bundle, Debian package and RPM.
8. Keeps `main`, `prerelease`, and `dev` aligned for the next promotion.
9. Stages and verifies the exact release assets privately.
10. Waits for release-note review and explicit publication approval.
11. Revalidates all four assets and publishes only the staged draft, preserving its reviewed
    title and notes.
12. Downloads the published assets and independently verifies their complete
    inventory, checksums, native metadata and embedded identities.
13. For prereleases, exercises production GitHub discovery, acquisition,
    confirmation, installation, and final identity from the pinned baseline.
14. Exercises public native-package updates on Ubuntu/Fedora and a public
    v1.8 bundle upgrade followed by installed GUI startup conversion.

`install.sh` is only an installer. It does not build the runtime or GUI.

## Native release assets

Every new release contains exactly four assets. For `2.0.0-beta.3` they are:

- `lg-buddy-2.0.0-beta.3-x86_64-unknown-linux-musl.tar.gz`
- `lg-buddy_2.0.0-beta.3-1_amd64.deb`
- `lg-buddy-2.0.0-beta.3-1.x86_64.rpm`
- `sha256sums.txt`

Native download filenames use upstream SemVer. Their embedded DEB/RPM version
remains `2.0.0~beta.3-1`, preserving native prerelease ordering. Separating these
forms avoids GitHub's [asset filename normalization](https://docs.github.com/en/rest/releases/assets#upload-a-release-asset),
which rewrites `~` during upload.

Official native releases support stable versions and `alpha`, `beta` or `rc`
prereleases, optionally followed by `.N` and then `.ci`. Promotion refuses
build metadata and forms whose Debian/RPM ordering could disagree with SemVer.
Published packages use revision `1`; `--revision 2` is for local packaging-only
upgrade tests and is installed manually. Both binaries retain the upstream
version/channel/commit identity independently of that packaging revision.

`scripts/release_assets.py` derives this inventory from the reviewed version.
It rejects missing/unexpected assets and incomplete, duplicate or mismatched
checksums. Non-installing native tools inspect package metadata and payloads;
both executables and each package manifest must match the bundle's exact
version, channel, target and commit. Verification never executes package
scriptlets or candidate binaries. Fedora 44 provides RPM 6 decoding and Debian
decoding for assembly, draft staging and publication.

Draft resumption uploads only missing expected assets. Existing assets must
match the local size and GitHub SHA-256 digest before publication. Published
releases are verification-only. Package assets are downloaded independently
after publication and checked against the same complete inventory. Historical
releases retain their original assets; this workflow does not backfill them.

PR CI runs the same complete assembly check against the real bundle and both
native package artifacts. The required `bundle-smoke-test` includes that job.
The response recorder's `--packages` option captures all four assets and their
download responses; its default still supports historical bundle-only releases.

## Public package and legacy canaries

`production-native-canary` runs on Ubuntu 24.04 and Fedora 44 after both stable
and prerelease publication. It selects an actual older published prerelease
with the complete native asset set as test input. The installed application
owns target discovery, confirmation, download, root revalidation and APT/DNF
installation; the harness does not substitute release HTTP responses. It
checks exact installed identity, native package verification, retained data,
normal-user GTK handoff and a fresh public update check. Sanitized GitHub
responses include all four assets. `result.json` records the baseline, target,
package digest, actual journey and outcome.

The first native release has no authentic older native package. Its canaries
install and verify that public package, then record **bootstrap**, with the
update journey explicitly unqualified. Bootstrap is not evidence of a passing
package update. Later prereleases qualify beta-to-beta, and a stable publication
qualifies prerelease-to-stable. Existing older native releases without an
eligible complete prerelease baseline cause a failure rather than a false
bootstrap result. Release review requires public results for both package
families and both update journeys.

`production-v1-bundle-canary` independently downloads the pinned published
`v1.8.0` bundle and the published candidate. The candidate's public asset
digests, checksum entries and identity are verified before use. The harness
deploys the historical payload, seeds supported stale 1.x selectors, and runs
the candidate bundle installer in an isolated prefix. Installation must preserve
the stale settings and credential bytes/ownership/modes. The installed GTK
application then converts them locally through its shared owner-only startup
operation, without starting a daemon. Overview, brightness and installed-update
handoff entries must all convert restored stale contents; a repeated start with
current contents must not rewrite the file. Version/runtime probes stay read-only.
There is no migration screen or acknowledgement.

Canaries use disposable environments, passwordless sudo for native transactions,
Xvfb/private session buses, synthetic retained credentials and no physical TV.
The legacy check skips historical pip and service activation. Its converted and
current GUI starts must show the same expected setup-required state because the
isolated fixture has no running service; this is not service readiness evidence.
These check public transport and product update journeys. Native desktop
qualification also exercises actual system/user service managers, polkit
cancellation/acceptance, installed GUI handoff, reboot and retained data in
disposable Ubuntu/Fedora desktop VMs. Record the source commit, environment,
package identities and results with the release PR or CI artifacts.
PR CI runs the same v1.8 installer/GTK/startup harness against its v2 candidate
bundle, alongside the retained v1.4 historical upgrade suite. No historical
release is republished or modified by these checks.

### Fedora automatic-update baseline transition

Fedora 44+ is the RPM automatic-update baseline; see the
[support policy](../README.md#distribution-support). The known beta.3/beta.4
helpers cannot replay newer DNF plans. For those exact baseline tag/commit
pairs and a later prerelease, the public canary verifies a manual RPM upgrade
and records `manual-transition`, `automatic_upgrade_qualified=false`. It never
converts arbitrary update failures or stable-release qualification to this
outcome. Once a newer baseline containing the fixed updater exists, the normal
automatic journey is required.

Before stable promotion, complete Fedora 44 desktop authorization, service and
reboot checks, plus an automatic upgrade between two published releases with
the fixed updater. Container checks, manual transitions and the historical
Fedora 43 desktop run do not complete these release checks.

## Release bundle identity

Every release archive contains `release-manifest.json` at the bundle root. The
schema-versioned JSON records the exact release tag, semantic version, release
channel, headless Rust target triple, GTK GUI target triple, and full lowercase commit SHA. Version, tag, and
channel must agree: stable SemVer maps to `stable`, prerelease SemVer maps to
`prerelease`, and the tag is exactly `v<version>`.

Schema 1 keeps the original five identity fields critical and adds
`gui_target` as a required extension for GUI-bearing bundles. Keeping it
non-critical lets the public v1.4 updater read the new manifest. Validators reject an
unsupported schema, duplicate JSON fields, missing identity fields, and unknown
critical fields. Unknown non-critical fields may be ignored for compatible
schema evolution. Official manifests use a deterministic field order and JSON
rendering.

The bundle builder derives version, channel, and commit from both executables;
it cannot package binaries whose embedded identities, targets, or `--version`
output disagree. The archive remains named for the static
`x86_64-unknown-linux-musl` runtime. The dynamic
`x86_64-unknown-linux-gnu` GUI is stored as
`docs/lg-buddy-gui-x86_64-unknown-linux-gnu`: that established namespace is
understood by the v1.4 updater. The named application icon is likewise carried
under `docs/` for compatibility. The installer publishes them as
`/usr/bin/lg-buddy-gui` and under the hicolor application-icon directory, and
safely restores GUI executable mode when an older updater extracted it as data.
Smoke validation checks the manifest before executing installer code and then
compares it with both the bundled and installed binary. Publishing validates
the manifest directly from each archive without extracting or executing archive
content. CI builds on Ubuntu 24.04, establishing GTK 4.14, libadwaita 1.5, and
a maximum GLIBC 2.39 symbol baseline. Fedora 44 and current Arch lanes install
and launch the same artifact under Xvfb, drive it without a pointer, inspect its
AT-SPI tree, and compare its light/dark and 1x/2x rendering; Ubuntu remains the
ABI baseline.

## Upgrade compatibility preflight

Release-bundle replacement is guarded by observed capability rather than a
distribution allowlist or provenance receipt. The initial runtime preflight
checks the installed mutable FHS topology, config discovery, ownership, path
types and mount boundaries, trusted system containment, integration override
state, and system/user service-manager availability before an updater performs
release or privilege-related effects. A verified candidate's binary performs a
second pass for its own installer requirements and trusted external ancestor
chain before privileged mutation.
The checker assigns each target an installer-operation policy so replacement,
directory mutation, recursive removal, exact drop-in, and candidate-input
requirements cannot silently lose their operation-specific safeguards.
User service checks follow an absolute `XDG_CONFIG_HOME`, falling back to
`~/.config` otherwise, as the installer and setup flow do. Setup helper and
polkit destinations may be absent on older installations; the preflight checks
that they can be safely created or replaced and requires their candidate payloads.
The machine binding is `/etc/lg-buddy/config-path`; only its absence permits
migration input from `/usr/lib/lg-buddy/config-path`. Preflight checks the new
binding destination and the user's discovery record before installation. Bundle
upgrades atomically seed the new binding before replacing the runtime, retaining
the legacy record for recovery. Ordinary GUI/CLI config discovery stays within
the invoking user's configuration home, including custom paths remembered there.

The installed GUI uses the same compatibility checks with its own installed
executable path; a source checkout or externally managed GUI cannot substitute
for the mutable release-bundle installation. GUI updates keep confirmation and
network work unprivileged, then use one graphical authorization for a bounded
system-installation helper. User configuration and user service operations
remain in the invoking user's process. CLI updates retain terminal confirmation
and their existing sudo behavior.

The extracted candidate exposes this second pass through the hidden
`upgrade-preflight` installer entrypoint. `install.sh --upgrade` invokes it
before sudo or installation writes, loads the existing config pointer and
settings without rewriting them, and never runs configuration, discovery, or
pairing. Upgrades remove the obsolete LG Buddy Python environment after
additional directory and mount checks, regardless of whether the saved profile
is native or stale. Installation leaves per-user configuration and credentials
untouched; the installed v2 application starts migration for stale profiles.
Fresh installations never create a Python environment. After replacing owned runtime and
integration files, the installer reloads system integrations before user
integrations and verifies that the installed binary matches the candidate.

This is intentionally a conservative and evolving refusal boundary. It does
not migrate legacy layouts, declare broad host support, or guarantee that a
later privileged operation cannot fail.

## Cross-version upgrade baseline

`v1.4.0-beta.2` is the first public updater-capable release and is the pinned
previous archive for the initial cross-version contract. Its
`x86_64-unknown-linux-musl` archive SHA-256 is
`883e6cb869cbe60988a195acac2e15864d904797edfefbb7d90052eff9a17d32`.
CI verifies that digest and the full release identity before extracting or
executing the baseline.

Versions before `v1.4.0-beta.2` do not contain `updates install`. They require
one normal manual installation of an updater-capable release before assisted
upgrades become available. Arbitrary historical upgrade support is not implied.

## Nix source selection

Nix configurations may select `main`, `prerelease`, or `dev` as the upstream
source according to the desired stability. The Nix lock file must continue to
pin an exact commit: the branch selects the update stream, not an implicitly
moving deployment.

This source selection is separate from LG Buddy's runtime `updates.channel`
setting, which controls GitHub release discovery for installed release bundles.
Use the matching [Nix channel outputs](nix.md#install-both-layers) for package
identity: `stable` for `main`, `prerelease` for that stream, and `dev` for
development. Locked source metadata does not identify the originally selected
branch. The prerelease stream reports stable identity while aligned with main;
development output aliases always retain development identity.

## Installing from a release bundle

End users can extract the release archive and run:

```bash
./install.sh
```

That path uses the bundled `lg-buddy` binary and does not require a Rust toolchain.
For a fresh installation it opens the installed GUI for TV pairing and subsequent
service setup. Failed or cancelled setup preserves desired settings. Both GUI
and CLI consume the [shared completion flow](onboarding.md); the GUI checks setup
readiness asynchronously on startup. Run it as the regular
user; the installer and GUI request only the elevation their system operations
need. For terminal setup, run `./install.sh --headless`; pass setup arguments after `--`. Existing TV
configurations retain their saved preferences.

To update an existing compatible release-bundle installation from an already
verified and extracted newer bundle, run as the installed user:

```bash
./install.sh --upgrade
```

An incompatible or legacy layout is refused rather than migrated. If a failure
occurs after installation writes begin, correct the reported cause and rerun the
same verified bundle with `--upgrade`.

## Installing locally built binaries

If you build `lg-buddy` and `lg-buddy-gui` yourself, install them by passing both
binary paths explicitly:

```bash
./install.sh \
  --runtime-binary ./target/x86_64-unknown-linux-musl/release/lg-buddy \
  --gui-binary ./target/x86_64-unknown-linux-gnu/release/lg-buddy-gui
```
