# Native release implementation validation

Recorded 2026-10-24 for [#136](https://github.com/Staphylococcus/LG_Buddy/issues/136).
This is local validation of the publication and canary implementation. No v2
tag, draft, release or public package was created for these checks.

## Tested inputs

The product fixture was built from
`cefd0b3510f36ddfb78366a23bdda03f939890fe`, using the test identity
`2.0.0-beta.1.ci`. The optimized musl runtime and GNU GTK binary came from the
same Ubuntu 24.04 build. Ubuntu 24.04 and Fedora 43 built the native packages
with source timestamp `1792805726`. The later parent commit `8e75baf` changes
only the RPM test's dependency bootstrap; #136 changes release tooling and tests.

SHA-256 identities:

```text
lg-buddy-2.0.0-beta.1.ci-x86_64-unknown-linux-musl.tar.gz
abe8d060870e9dbab143c86e92860c72596e16a492102074a634dc3aebefb92e
lg-buddy_2.0.0~beta.1.ci-1_amd64.deb
8a1080f0dc21465a83ec2986d2b9cf2e2d5d54fb6aa310891aa12520648aa799
lg-buddy-2.0.0~beta.1.ci-1.x86_64.rpm
79c9f558cd0bf10e99fdddaf38f8b8e5d3440fa8a3835c28f14f25b70f640083
```

## Results

| Check | Result |
| --- | --- |
| Release contracts | 97 focused tests passed across inventory/native identity, publication/retry, response recording, canary input/selection, manifest, promotion and mock contracts. Unsupported native version forms refuse promotion before merge. |
| Complete native assembly | Both native metadata tuples, both embedded executables and both package manifests matched the bundle's exact identity. Checksums covered all three payloads; staging/publication dry runs accepted exactly four assets. |
| Native lifecycle | The actual Debian and RPM test packages passed install, packaging-revision upgrade, reinstall, installed GTK, owner startup migration, removal and retained-data checks. Revision 2 was a private fixture. |
| Refusal boundaries | Missing/unexpected assets, incomplete uploads, duplicate or incorrect checksums, corrupt public downloads, wrong native metadata and wrong embedded commits refused. Checksum failure preceded native decoding. |
| APT and DNF regression | Real native updates passed root digest refusal, declined confirmation, verified installation, retained data and normal-user GTK handoff. Both PTY outcomes also produced the logs used by the public canaries. |
| Published v1.8 baseline | Downloaded `v1.8.0` digest `c9e19ce2c2a265bda253180096c1babc772a93d980f1080436b321c638d6b9a4` matched its published asset. Its product commit is `ed0b740ee3873177682efe2d1f6ae7930e847aef`. |
| Legacy upgrade/startup | Actual v1.8 payload installation, candidate bundle upgrade and GTK opening preserved the seeded stale configuration and credentials. The installed screen daemon converted only the supported platform/backend selectors as UID 1000, preserving preferences, bytes and access metadata. GTK remained open without a migration acknowledgement. |
| Minimal Fedora runtime prerequisite | A clean image initially lacked GLES. DNF pulled its provider through the RPM dependency; native lifecycle, installed GTK, owner startup migration and retention passed with weak dependencies disabled. All nine native RPM input/ordering tests passed. |

The native updater regressions use debug-only GitHub transport fixtures and
passwordless sudo. The legacy journey uses an isolated prefix, Xvfb/private
session bus and synthetic retained credentials. Stale selectors are seeded
after deploying v1.8 because that installer does not repair retired Python
environments. Historical pip/service activation is skipped; the real installed
owner daemon is started directly. No physical TV or new VM was used.

## Outstanding production qualification

The public workflows are wired but have not run against published v2 native
assets. The first package release records bootstrap installation, not a passing
update journey. #136 remains unqualified until Ubuntu and Fedora have recorded
published beta-to-beta and prerelease-to-stable updates, plus the public v1.8
bundle/candidate startup journey. Those release executions retain the existing
reviewed-promotion and publication-approval boundaries.

Artifacts, logs and isolated legacy evidence are retained on the validation
host under `~/.local/state/lg-buddy-validation/release-136-20261024/`.
`exact-cefd0b3/` contains the complete test release asset set;
`legacy-runs/final/` records the installed GUI/startup journey.
