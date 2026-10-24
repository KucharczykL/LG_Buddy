# Native release implementation validation

Recorded 2026-10-24 for [#136](https://github.com/Staphylococcus/LG_Buddy/issues/136).
This is local validation of the publication and canary implementation. No v2
tag, draft, release or public package was created for these checks.

## Tested inputs

The initial product fixture was built from
`cefd0b3510f36ddfb78366a23bdda03f939890fe`, using the test identity
`2.0.0-beta.1.ci`. The optimized musl runtime and GNU GTK binary came from the
same Ubuntu 24.04 build. Ubuntu 24.04 and Fedora 43 built the native packages
with source timestamp `1792805726`. The later parent commit `8e75baf` changes
only the RPM test's dependency bootstrap. The review follow-up also adds the
shared owner-only conversion to normal GUI startup.

SHA-256 identities:

```text
lg-buddy-2.0.0-beta.1.ci-x86_64-unknown-linux-musl.tar.gz
abe8d060870e9dbab143c86e92860c72596e16a492102074a634dc3aebefb92e
lg-buddy_2.0.0~beta.1.ci-1_amd64.deb
8a1080f0dc21465a83ec2986d2b9cf2e2d5d54fb6aa310891aa12520648aa799
lg-buddy-2.0.0~beta.1.ci-1.x86_64.rpm
79c9f558cd0bf10e99fdddaf38f8b8e5d3440fa8a3835c28f14f25b70f640083
```

The reviewed GUI fix was rebuilt from
`48059d36027fbaa6d1f87853f9124fe192508b2b`, retaining the same test version,
optimized targets and distro builders. Its native source timestamp is
`1792818718`. The complete rebuilt asset set passed identity/checksum validation
and both publisher dry runs:

```text
bundle: f3643b4a24194129502e07596f95ba690ae0163b3aa22d352a4787fe2baf6730
Debian: f696841fe8d5fd7b29b1054278e8a72b927f7fc78e2c573bfccd4b4868115ad4
RPM:    6cc5fb95f84abf0c1b0a87adbd48f7be16d9d3947197c7f4280c99c1cba669c7
```

## Results

| Check | Result |
| --- | --- |
| Release contracts | 100 focused tests passed across inventory/native identity, publication/retry, response recording, canary input/selection, manifest, promotion and mock contracts. Unsupported native version forms refuse promotion before merge. |
| Complete native assembly | Both native metadata tuples, both embedded executables and both package manifests matched the bundle's exact identity. Checksums covered all three payloads; staging/publication dry runs accepted exactly four assets. |
| Native lifecycle | The actual Debian and RPM test packages passed install, packaging-revision upgrade, reinstall, installed GTK, owner startup migration, removal and retained-data checks. Revision 2 was a private fixture. |
| Refusal boundaries | Missing/unexpected assets, incomplete uploads, duplicate or incorrect checksums, corrupt public downloads, wrong native metadata and wrong embedded commits refused. Checksum failure preceded native decoding. |
| APT and DNF regression | Real native updates passed root digest refusal, declined confirmation, verified installation, retained data and normal-user GTK handoff. Both PTY outcomes also produced the logs used by the public canaries. |
| Published v1.8 baseline | Downloaded `v1.8.0` digest `c9e19ce2c2a265bda253180096c1babc772a93d980f1080436b321c638d6b9a4` matched its published asset. Its product commit is `ed0b740ee3873177682efe2d1f6ae7930e847aef`. |
| Legacy upgrade/GUI startup | The rebuilt candidate's installer preserved stale settings and credentials. Its installed GUI alone converted only the supported selectors as UID 1000 through overview, brightness and update handoff entries; no daemon was launched. Repeated current startup did not rewrite settings, and version/runtime probes preserved stale settings. Credentials and access metadata remained unchanged throughout. |
| Minimal Fedora runtime prerequisite | A clean image initially lacked GLES. DNF pulled its provider through the RPM dependency; native lifecycle, installed GTK, owner startup migration and retention passed with weak dependencies disabled. All nine native RPM input/ordering tests passed. |

The native updater regressions use debug-only GitHub transport fixtures and
passwordless sudo. The legacy journey uses an isolated prefix, Xvfb/private
session bus and synthetic retained credentials. Stale selectors are seeded
after deploying v1.8 because that installer does not repair retired Python
environments. Historical pip/service activation is skipped. Converted and current
GUI starts must show the same explicit service-unavailable setup state; this is
not evidence of service readiness. No physical TV or new VM was used.

## Review follow-up

- The original GUI receipt used a direct daemon start and did not qualify GUI
  startup conversion. The stricter harness rejects that old candidate, then
  passes all three installed GUI entries plus an idempotent current start on
  the rebuilt candidate, without a migration screen or acknowledgement.
- A real Debian package rebuilt with nonempty `conffiles` now fails publication
  verification, matching the installed updater's control-archive checks. The
  unchanged package remains accepted.
- In minimal Ubuntu, the public canary's installed-file verifier rejects the
  missing documentation manifest. After opting the complete LG Buddy docs back
  in and reinstalling, verification passes. Corrupting a checksummed installed
  file then refuses despite `dpkg --verify` exiting zero; restoring it passes.

Changed Rust files pass formatting checks. A repository-wide formatting check
reports pre-existing differences in other files, which this slice leaves alone.

## Outstanding production qualification

The public workflows are wired but have not run against published v2 native
assets. The first package release records bootstrap installation, not a passing
update journey. #136 remains unqualified until Ubuntu and Fedora have recorded
published beta-to-beta and prerelease-to-stable updates, plus the public v1.8
bundle/candidate startup journey. Those release executions retain the existing
reviewed-promotion and publication-approval boundaries.

Artifacts, logs and isolated legacy evidence are retained on the validation
host under `~/.local/state/lg-buddy-validation/release-136-20261024/`.
`exact-cefd0b3/` contains the initial complete test release asset set;
`legacy-runs/final/` retains the superseded daemon-assisted GUI receipt.
`review-fixes/assembled/` contains the rebuilt complete set, and
`review-fixes/installed-gui-final/` records the actual GUI-startup journey.
The corresponding `logs/review-*` files retain the positive and refusal checks.
