# DNF stored replay validation

Recorded 2026-10-24 for [#334](https://github.com/Staphylococcus/LG_Buddy/issues/334).
This records the upstream-fix slice at `1ddd6f9`, before the platform policy
change below. These are disposable native integration results; published-release and real
desktop qualification remain separate under #136.

## Upstream fix and application changes

DNF's [upstream fix](https://github.com/rpm-software-management/dnf5/pull/2461)
prefixes stored repository IDs to avoid collisions with configured repositories.
It is listed in [DNF 5.3.0.0](https://github.com/rpm-software-management/dnf5/releases/tag/5.3.0.0).
The tested Fedora 44 DNF 5.4.3.0 contains that merged fix
(`08cabceeda51a9442724d8f33a991090cf203ce8`); 5.3.0.0 itself was not executed.

The application accepts exactly one `@stored_transaction(...)` wrapper and
continues to require the exact verified local LG Buddy upgrade. Wrapped
command-line dependencies, malformed/nested wrappers and wrapped `@System`
replacements are refused, alongside existing destructive-action restrictions.

Newer DNF classifies the stored dependency repositories as command-line
repositories. Its replay skipped OpenPGP checks despite `*.gpgcheck=True` in
the standalone probe. The helper therefore requires signatures **and** digests
for each repository RPM before replay, using `rpmkeys --checksig` with
`_pkgverify_level all` and `_pkgverify_flags 0x0`. It uses the existing system
keyring and does not import keys. The exact LG Buddy RPM retains its independent
GitHub release/digest verification boundary.

## Measured journeys

| Environment | Journey | Result |
| --- | --- | --- |
| Fedora 44; DNF/libdnf5 `5.4.3.0-2.fc44` | Standalone store/replay of absent `libglvnd-gles` | Passed with prefixed repository IDs; DNF's signature-skipping warning motivated the helper check above. |
| Same Fedora 44 | Application updater adds absent `libglvnd-gles-1:1.7.0-9.fc44.x86_64` | Passed; dependency payload verified, exact app identity/ownership checked, saved data retained and installed GUI relaunched as the normal user. |
| Fedora 43; DNF/libdnf5 `5.2.18.0-5.fc43` | Application update with dependencies already installed | Passed before the Fedora 44+ policy; current automatic RPM updates on 43 are refused. |
| Same Fedora 43 | Original standalone dependency-changing store/replay | Failed with mirror 404s despite the stored RPM being present. |
| Same Fedora 43 | Disposable plan with upstream-equivalent repository prefixes, after strict `rpmkeys` verification | Passed for `libglvnd-gles-1:1.7.0-8.fc43.x86_64`; assessment only, not an implemented application workaround. |

The native helper tests accepted a genuine Fedora-signed RPM and refused
unsigned, payload-tampered and symlink inputs. The application smokes also
checked privileged release-digest refusal, version-lock refusal, declined
confirmation and settings/credential/machine-binding bytes, UID/GID and modes.
The full `cargo test --locked -p lg-buddy` run passed, including 1,577 library
tests, integration tests and all 155 behavior scenarios; one existing library
test remained ignored. Workflow lint and Python compilation also passed.

The application tests used the existing debug transport baseline at `0.0.0-1`
with this slice's helper, and the unchanged immutable public beta.4 candidate
(`60f4d65b460e37a1a4120a0d665e48cbd8f26e55`, native `2.0.0~beta.4-1`).
The candidate RPM SHA-256 was
`eb1fe367253c901170359e9a7cc05744d4761c5d5a153fb13bd4c08a1f1a9976`.
HTTP transport and scoped sudo authorization were test substitutes. This does
not establish a newly published update or requalify real polkit/service/reboot
behavior. Containers were limited to 1–2 GiB RAM with no extra swap allowance;
no VM was started.

## Fedora 43 decision and remaining qualification

The user selected **Fedora 44+ as the automatic RPM update baseline** rather
than shipping the Fedora 43 compatibility rewrite. Current GUI/CLI preparation
and the root helper refuse unsupported RPM platforms with manual-update
instructions. Fedora 43 refusal is now a required CI check. See the
[distribution support policy](distribution-support.md) for all distro baselines,
Fedora 43 instructions and the one-time manual transition for early v2 betas.

The successful isolated Fedora 43 prefix probe above remains historical
assessment evidence. It does not establish a shipped workaround or current
Fedora 43 automatic-update support. Historical desktop/public beta results are
preserved under their original distribution versions.

The new Fedora 44 CI job requires a genuinely absent repository dependency and
exercises the signature gate using native RPM tools. CI execution and public
qualification of a release containing these changes remain outstanding;
#334 remains open.

The subsequent platform-policy slice passed the full runtime suite (1,579
library tests, all integration tests and 155 behavior scenarios; one existing
library test ignored). Its Fedora 44 native handoff added the absent GLES
dependency and retained user state. Fedora 43 CLI and root-helper checks
refused with manual-update guidance, with no release HTTP requests, mutation
marker or package/user-state changes. Fedora 44 package builds were byte
identical on repetition, and its package contract and lifecycle/startup checks
passed. Workflow lint and canary/release contract tests passed locally.

An authentic public beta.3 RPM on Fedora 44, given the authentic public beta.4
RPM, reproduced `DNF did not select the verified LG Buddy upgrade` before the
mutation marker and retained beta.3. This corroborates the early-beta manual
transition documented in the support guide. The controlled transition is
limited to the known immutable beta.3/beta.4 identities and a newer prerelease;
it cannot qualify a stable release or hide failures from a newer baseline.
No new public helper release, Fedora 44 desktop VM or public automatic-upgrade
journey was created by these checks.

Platform-policy receipts and logs are retained separately under
`~/.local/state/lg-buddy-validation/issue334-distro-policy-20261024/`.

Raw logs, original/prefixed transaction JSON, tool versions and helper test
artifacts are retained locally under
`~/.local/state/lg-buddy-validation/issue334-dnf-upstream-20261024/`.
