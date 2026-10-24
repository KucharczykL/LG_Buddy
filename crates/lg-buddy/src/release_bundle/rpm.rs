//! RPM decoding shares the release trust boundary with Debian packages.
use super::native_package::{self, NativeFormat};
use super::*;

pub(crate) use super::native_package::VerifiedNativePackage as VerifiedRpmPackage;

pub(crate) fn native_version(version: &Version) -> Result<String, BundleAcquisitionError> {
    let parts: Vec<_> = version.pre.as_str().split('.').collect();
    let valid = version.pre.is_empty()
        || (matches!(parts[0], "alpha" | "beta" | "rc")
            && match &parts[1..] {
                [] => true,
                [number] | [number, "ci"] => {
                    !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
                }
                _ => false,
            });
    if !version.build.is_empty() || !valid {
        return Err(BundleAcquisitionError::ReleaseMetadata(
            "RPM prereleases require alpha, beta or rc, optionally .N and then .ci".into(),
        ));
    }
    Ok(format!("{}-1", version.to_string().replacen('-', "~", 1)))
}

pub(super) fn asset_name(version: &Version) -> Result<String, BundleAcquisitionError> {
    Ok(format!(
        "lg-buddy-{}.x86_64.rpm",
        native_version(version)?.replace('~', "-")
    ))
}

pub(crate) fn resolve_identity(
    release: &ReleaseInfo,
) -> Result<ReleaseIdentity, BundleAcquisitionError> {
    native_package::resolve_identity(NativeFormat::Rpm, release)
}
pub(crate) fn acquire(release: &ReleaseInfo) -> Result<VerifiedRpmPackage, BundleAcquisitionError> {
    native_package::acquire(NativeFormat::Rpm, release)
}
pub(crate) fn receive(
    expected: &ReleaseIdentity,
    input: impl Read,
    cache: &Path,
) -> Result<native_package::RootNativePackage, BundleAcquisitionError> {
    native_package::receive(NativeFormat::Rpm, expected, input, cache)
}

fn header_paths(text: &str) -> Result<HashSet<String>, BundleAcquisitionError> {
    let mut paths = HashSet::new();
    for line in text.lines() {
        let (path, flags) = line
            .split_once('\t')
            .ok_or_else(|| archive_error("Malformed RPM file inventory"))?;
        // Only ordinary files, documentation and licenses. In particular, reject
        // config/ghost entries that could claim mutable files absent from payload.
        let flags = flags.parse::<u32>().map_err(archive_error)?;
        let raw = path
            .strip_prefix('/')
            .ok_or_else(|| archive_error("Nonabsolute RPM path"))?;
        let normalized = normalize_archive_path(raw.as_bytes(), EntryType::Regular)?;
        if flags & !(2 | 128) != 0
            || !raw.starts_with("usr/")
            || normalized == "usr/lib/lg-buddy/config-path"
            || paths.len() >= MAX_ARCHIVE_ENTRIES
            || !paths.insert(normalized)
        {
            return Err(archive_error("Unsafe or duplicate RPM file inventory"));
        }
    }
    Ok(paths)
}

pub(crate) fn verify_package(
    file: &File,
    expected: &ReleaseIdentity,
) -> Result<(), BundleAcquisitionError> {
    let fields = native_package::decode(
        file,
        "/usr/bin/rpm",
        &[
            "--query",
            "--package",
            "--queryformat",
            "%{NAME}\n%{EPOCHNUM}\n%{VERSION}-%{RELEASE}\n%{ARCH}\n",
            "-",
        ],
        MAX_MANIFEST_BYTES,
    )?;
    let wanted = format!(
        "lg-buddy\n0\n{}\nx86_64\n",
        native_version(expected.version())?
    );
    if fields != wanted.as_bytes() {
        return Err(BundleAcquisitionError::Manifest(
            "RPM name, epoch, version or architecture disagrees with the release".into(),
        ));
    }
    let inventory = native_package::decode(
        file,
        "/usr/bin/rpm",
        &[
            "--query",
            "--package",
            "--queryformat",
            "[%{FILENAMES}\t%{FILEFLAGS}\n]",
            "-",
        ],
        MAX_ARCHIVE_METADATA_BYTES,
    )?;
    let inventory = String::from_utf8(inventory).map_err(archive_error)?;
    let paths = header_paths(&inventory)?;
    let data = native_package::decode(
        file,
        "/usr/bin/rpm2archive",
        &["-n", "-"],
        MAX_ARCHIVE_EXPANDED_BYTES,
    )?;
    if native_package::verify_data(&data, expected)? != paths {
        return Err(archive_error("RPM header and payload paths disagree"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_download_names_do_not_contain_native_tildes() {
        assert_eq!(
            asset_name(&Version::parse("2.0.0-beta.2.ci").unwrap()).unwrap(),
            "lg-buddy-2.0.0-beta.2.ci-1.x86_64.rpm"
        );
        assert_eq!(
            asset_name(&Version::parse("2.0.0").unwrap()).unwrap(),
            "lg-buddy-2.0.0-1.x86_64.rpm"
        );
    }
    #[test]
    fn versions_preserve_supported_semver_order() {
        for (version, wanted) in [
            ("2.0.0", "2.0.0-1"),
            ("2.0.0-beta.2.ci", "2.0.0~beta.2.ci-1"),
            ("2.0.0-rc", "2.0.0~rc-1"),
        ] {
            assert_eq!(
                native_version(&Version::parse(version).unwrap()).unwrap(),
                wanted
            );
        }
        for version in [
            "2.0.0-alpha.ci",
            "2.0.0-dev",
            "2.0.0+build",
            "2.0.0-beta.1.extra",
        ] {
            assert!(native_version(&Version::parse(version).unwrap()).is_err());
        }
    }
    #[test]
    fn header_refuses_mutable_ghost_and_duplicate_files() {
        assert!(header_paths("/usr/bin/lg-buddy\t0\n/usr/share/doc/lg-buddy/a\t2\n/usr/share/licenses/lg-buddy/LICENSE\t128\n").is_ok());
        for text in [
            "/etc/a\t0\n",
            "/usr/a\t1\n",
            "/usr/a\t64\n",
            "/usr/lib/lg-buddy/config-path\t0\n",
            "/usr/a\t0\n/usr/a\t0\n",
            "usr/a\t0\n",
            "/usr/../etc/a\t0\n",
        ] {
            assert!(header_paths(text).is_err(), "{text}");
        }
    }
    fn source(bytes: &[u8]) -> super::super::tests::FakeSource {
        use super::super::tests::{release_asset, release_info, stable_identity, FakeSource};
        let name = asset_name(stable_identity().version()).unwrap();
        let checksum = format!("{:x}  {name}\n", Sha256::digest(bytes)).into_bytes();
        FakeSource {
            fresh_release: release_info(vec![
                release_asset(1, &name, bytes),
                release_asset(2, CHECKSUM_ASSET_NAME, &checksum),
            ]),
            payloads: HashMap::from([(1, bytes.to_vec()), (2, checksum)]),
            commit: stable_identity().commit().into(),
        }
    }
    fn fixture(directory: &Path, package: &str, gui_commit: &str, extra: &str) -> Vec<u8> {
        use super::super::tests::{embedded_identity_binary, stable_identity};
        let root = directory.join("payload/usr/bin");
        fs::create_dir_all(&root).unwrap();
        let expected = stable_identity();
        for (name, identity) in [
            ("lg-buddy", expected.clone()),
            (
                "lg-buddy-gui",
                ReleaseIdentity {
                    target: GUI_TARGET.into(),
                    commit: gui_commit.into(),
                    ..expected
                },
            ),
        ] {
            let path = root.join(name);
            fs::write(&path, embedded_identity_binary(&identity)).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let spec = directory.join("fixture.spec");
        fs::write(&spec, format!("Name: {package}\nVersion: 1.4.0\nRelease: 1\nEpoch: 0\nSummary: Decoder fixture\nLicense: GPL-3.0-only\nBuildArch: x86_64\n%global debug_package %{{nil}}\n%global __os_install_post %{{nil}}\n%global _build_id_links none\n%description\nDecoder fixture\n%install\nmkdir -p %{{buildroot}}/usr/bin\ncp {}/lg-buddy* %{{buildroot}}/usr/bin/\n%pre\nexit 66\n%files\n%defattr(-,root,root,-)\n/usr/bin/lg-buddy\n/usr/bin/lg-buddy-gui\n{extra}\n", root.display())).unwrap();
        let status = std::process::Command::new("/usr/bin/rpmbuild")
            .args([
                "--define",
                &format!("_topdir {}", directory.join("build").display()),
                "--define",
                "_rpmformat 6",
                "--define",
                "_smp_build_ncpus 1",
                "-bb",
            ])
            .arg(&spec)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        fs::read(directory.join(format!("build/RPMS/x86_64/{package}-1.4.0-1.x86_64.rpm"))).unwrap()
    }
    #[test]
    fn native_rpm_acquisition_and_root_receipt_recheck_bytes_without_running_scriptlets() {
        use super::super::tests::{release_info, stable_identity, temp_dir};
        if !Path::new("/usr/bin/rpmbuild").is_file() {
            eprintln!("native RPM decoding runs on Fedora");
            return;
        }
        let directory = temp_dir("rpm-acquisition");
        let bytes = fixture(&directory, "lg-buddy", stable_identity().commit(), "");
        let source = source(&bytes);
        let acquired = native_package::acquire_with(
            NativeFormat::Rpm,
            &release_info(vec![]),
            &directory.join("user-cache"),
            &source,
        )
        .unwrap();
        assert_eq!(acquired.identity, stable_identity());
        let received = native_package::receive_with(
            NativeFormat::Rpm,
            &stable_identity(),
            acquired.input().unwrap(),
            &directory.join("root-cache"),
            &source,
        )
        .unwrap();
        assert_eq!(fs::read(&received.path).unwrap(), bytes);
        assert_eq!(fs::metadata(&received.path).unwrap().mode() & 0o777, 0o600);
        drop(received);
        for bad in [
            bytes[..bytes.len() - 1].to_vec(),
            [bytes.as_slice(), b"extra"].concat(),
            vec![0; bytes.len()],
        ] {
            assert!(native_package::receive_with(
                NativeFormat::Rpm,
                &stable_identity(),
                bad.as_slice(),
                &directory.join("root-cache"),
                &source
            )
            .is_err());
        }
        drop(acquired);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn native_rpm_refuses_wrong_owner_identity_and_ghost_configuration() {
        use super::super::tests::{stable_identity, temp_dir};
        if !Path::new("/usr/bin/rpmbuild").is_file() {
            return;
        }
        for (name, commit, extra) in [
            ("foreign", stable_identity().commit(), ""),
            ("lg-buddy", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", ""),
            (
                "lg-buddy",
                stable_identity().commit(),
                "%ghost /etc/lg-buddy/config-path",
            ),
        ] {
            let directory = temp_dir("rpm-invalid");
            let bytes = fixture(&directory, name, commit, extra);
            let path = directory.join("invalid.rpm");
            fs::write(&path, bytes).unwrap();
            assert!(verify_package(&File::open(path).unwrap(), &stable_identity()).is_err());
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
