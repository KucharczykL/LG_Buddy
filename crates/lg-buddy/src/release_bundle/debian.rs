//! Verified Debian release payloads. Transport, tag and digest rules are shared
//! with release bundles; dpkg-deb decodes the native format without installing it.

use super::*;
use std::io::Cursor;
use std::process::{Command, Stdio};

#[derive(Debug)]
pub(crate) struct VerifiedDebianPackage {
    pub(crate) identity: ReleaseIdentity,
    file: File,
    _staging: StagingDirectory,
}

impl VerifiedDebianPackage {
    pub(crate) fn input(&self) -> Result<File, BundleAcquisitionError> {
        rewind_clone(&self.file)
    }
}

pub(crate) struct RootDebianPackage {
    pub(crate) path: PathBuf,
    _staging: StagingDirectory,
}

pub(crate) fn native_version(version: &Version) -> Result<String, BundleAcquisitionError> {
    let parts: Vec<_> = version.pre.as_str().split('.').collect();
    let valid = version.pre.is_empty()
        || (matches!(parts[0], "alpha" | "beta" | "rc")
            && match &parts[1..] {
                [] | ["ci"] => true,
                [number] | [number, "ci"] => {
                    !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
                }
                _ => false,
            });
    if !version.build.is_empty() || !valid {
        return Err(BundleAcquisitionError::ReleaseMetadata(
            "Debian prereleases require alpha, beta or rc, optionally .N and .ci".into(),
        ));
    }
    Ok(format!("{}-1", version.to_string().replacen('-', "~", 1)))
}

fn asset_name(version: &Version) -> Result<String, BundleAcquisitionError> {
    Ok(format!("lg-buddy_{}_amd64.deb", native_version(version)?))
}

fn resolve_with<S: GitHubSource>(
    release: &ReleaseInfo,
    source: &S,
) -> Result<ResolvedRelease, BundleAcquisitionError> {
    let release = fresh_release_with(release, source)?;
    select_named_release_assets(&release, &asset_name(release.version())?)?;
    let identity = resolve_tag_identity(&release, source)?;
    Ok(ResolvedRelease { release, identity })
}

pub(crate) fn resolve_identity(
    release: &ReleaseInfo,
) -> Result<ReleaseIdentity, BundleAcquisitionError> {
    resolve_with(release, &UreqGitHubSource::new()).map(|resolved| resolved.identity)
}

pub(crate) fn acquire(
    release: &ReleaseInfo,
) -> Result<VerifiedDebianPackage, BundleAcquisitionError> {
    acquire_with(
        release,
        &acquisition_cache_root_from_env()?,
        &UreqGitHubSource::new(),
    )
}

fn acquire_with<S: GitHubSource>(
    release: &ReleaseInfo,
    cache: &Path,
    source: &S,
) -> Result<VerifiedDebianPackage, BundleAcquisitionError> {
    let staging = StagingDirectory::create(cache)?;
    let resolved = resolve_with(release, source)?;
    let selected =
        select_named_release_assets(&resolved.release, &asset_name(resolved.release.version())?)?;
    verify_published_digest(source, &staging, &selected)?;
    let path = staging.path.join(selected.archive.name());
    let downloaded = source.download_asset(selected.archive, &path, MAX_ARCHIVE_BYTES)?;
    pin_download_file(&path, &downloaded.file)?;
    verify_download(selected.archive, &downloaded.facts)?;
    verify_package(&downloaded.file, &resolved.identity)?;
    Ok(VerifiedDebianPackage {
        identity: resolved.identity,
        file: downloaded.file,
        _staging: staging,
    })
}

fn verify_published_digest<S: GitHubSource>(
    source: &S,
    staging: &StagingDirectory,
    selected: &SelectedAssets<'_>,
) -> Result<(), BundleAcquisitionError> {
    let path = staging.path.join(CHECKSUM_ASSET_NAME);
    let checksum = source.download_asset(selected.checksum, &path, MAX_CHECKSUM_BYTES)?;
    pin_download_file(&path, &checksum.file)?;
    verify_download(selected.checksum, &checksum.facts)?;
    let published = published_archive_digest(&checksum.file, selected.archive.name())?;
    if published != required_github_digest(selected.archive)? {
        return Err(BundleAcquisitionError::Checksum(
            "Debian package and published checksum disagree".into(),
        ));
    }
    Ok(())
}

/// Reobserve the official release under privilege, then receive only the exact
/// published bytes into a root-owned staging directory. No caller file path or
/// caller-provided digest grants authority to APT.
pub(crate) fn receive(
    expected: &ReleaseIdentity,
    input: impl Read,
    cache: &Path,
) -> Result<RootDebianPackage, BundleAcquisitionError> {
    receive_with(expected, input, cache, &UreqGitHubSource::new())
}

fn receive_with<S: GitHubSource>(
    expected: &ReleaseIdentity,
    input: impl Read,
    cache: &Path,
    source: &S,
) -> Result<RootDebianPackage, BundleAcquisitionError> {
    let selected_release = ReleaseInfo::from_github(
        expected.version().clone(),
        expected.channel(),
        format!(
            "https://github.com/{REPOSITORY_OWNER}/{REPOSITORY_NAME}/releases/tag/{}",
            expected.release_tag()
        ),
        expected.release_tag().to_string(),
        vec![],
    );
    let staging = StagingDirectory::create(cache)?;
    let resolved = resolve_with(&selected_release, source)?;
    if resolved.identity != *expected {
        return Err(BundleAcquisitionError::ReleaseMetadata(
            "Debian release changed after confirmation".into(),
        ));
    }
    let selected =
        select_named_release_assets(&resolved.release, &asset_name(expected.version())?)?;
    verify_published_digest(source, &staging, &selected)?;
    let path = staging.path.join(selected.archive.name());
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let mut reader = input.take(selected.archive.size() + 1);
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        bytes += count as u64;
    }
    verify_download(
        selected.archive,
        &DownloadFacts {
            bytes,
            sha256: format!("{:x}", hasher.finalize()),
        },
    )?;
    file.flush()?;
    verify_package(&file, expected)?;
    Ok(RootDebianPackage {
        path,
        _staging: staging,
    })
}

fn decode(file: &File, arguments: &[&str], limit: u64) -> Result<Vec<u8>, BundleAcquisitionError> {
    let mut child = Command::new("/usr/bin/dpkg-deb")
        .args(arguments)
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::from(rewind_clone(file)?))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .unwrap()
        .take(limit + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > limit {
        let _ = child.kill();
    }
    let status = child.wait()?;
    read?;
    if bytes.len() as u64 > limit || !status.success() {
        return Err(BundleAcquisitionError::Archive(
            "dpkg-deb failed or exceeded the Debian payload limit".into(),
        ));
    }
    Ok(bytes)
}

pub(crate) fn verify_package(
    file: &File,
    expected: &ReleaseIdentity,
) -> Result<(), BundleAcquisitionError> {
    let control = decode(file, &["--ctrl-tarfile", "-"], MAX_ARCHIVE_METADATA_BYTES)?;
    verify_control(&control)?;
    let fields = decode(
        file,
        &["--field", "-", "Package", "Version", "Architecture"],
        MAX_MANIFEST_BYTES,
    )?;
    let expected_fields = format!(
        "Package: lg-buddy\nVersion: {}\nArchitecture: amd64\n",
        native_version(expected.version())?
    );
    if fields != expected_fields.as_bytes() {
        return Err(BundleAcquisitionError::Manifest(
            "Debian package name, version or architecture does not match the release".into(),
        ));
    }
    let data = decode(file, &["--fsys-tarfile", "-"], MAX_ARCHIVE_EXPANDED_BYTES)?;
    verify_data(&data, expected)
}

fn verify_control(data: &[u8]) -> Result<(), BundleAcquisitionError> {
    let mut archive = Archive::new(Cursor::new(data));
    let mut seen = HashSet::new();
    for entry in archive.entries().map_err(archive_error)? {
        let entry = entry.map_err(archive_error)?;
        let raw = entry.path_bytes();
        let raw = raw.strip_prefix(b"./").unwrap_or(&raw);
        if (raw.is_empty() || raw == b".") && entry.header().entry_type() == EntryType::Directory {
            continue;
        }
        let path = normalize_archive_path(raw, entry.header().entry_type())?;
        if seen.len() >= 64
            || !seen.insert(path.clone())
            || entry.header().entry_type() != EntryType::Regular
            || path.contains('/')
            || (path == "conffiles" && entry.size() != 0)
        {
            return Err(archive_error(
                "Debian control metadata contains links, duplicates or package-owned configuration",
            ));
        }
    }
    Ok(())
}

fn verify_data(data: &[u8], expected: &ReleaseIdentity) -> Result<(), BundleAcquisitionError> {
    let mut archive = Archive::new(Cursor::new(data));
    let mut seen = HashSet::new();
    let mut binaries = HashSet::new();
    for entry in archive.entries().map_err(archive_error)? {
        let mut entry = entry.map_err(archive_error)?;
        let kind = entry.header().entry_type();
        let mode = entry.header().mode().map_err(archive_error)?;
        if !matches!(kind, EntryType::Regular | EntryType::Directory)
            || entry.header().uid().map_err(archive_error)? != 0
            || entry.header().gid().map_err(archive_error)? != 0
            || !matches!(mode, 0o644 | 0o755)
            || (kind == EntryType::Directory && mode != 0o755)
        {
            return Err(archive_error(
                "Debian payload has unsafe types, owners or modes",
            ));
        }
        let raw = entry.path_bytes();
        let raw = raw.strip_prefix(b"./").unwrap_or(&raw);
        if raw.is_empty() || raw == b"." {
            if kind == EntryType::Directory {
                continue;
            }
            return Err(archive_error("Debian root entry is not a directory"));
        }
        let path = normalize_archive_path(raw, kind)?;
        if seen.len() >= MAX_ARCHIVE_ENTRIES
            || !seen.insert(path.clone())
            || !(path == "usr" || path.starts_with("usr/"))
            || path == "usr/lib/lg-buddy/config-path"
            || entry.size() > MAX_ARCHIVE_FILE_BYTES
        {
            return Err(archive_error(format!(
                "unsafe Debian payload entry: {path}"
            )));
        }
        if path == "usr/bin/lg-buddy" || path == "usr/bin/lg-buddy-gui" {
            if kind != EntryType::Regular || entry.header().mode().map_err(archive_error)? != 0o755
            {
                return Err(archive_error("Debian binary must be a regular executable"));
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            let mut identity = expected.clone();
            if path.ends_with("-gui") {
                identity.target = GUI_TARGET.into();
            }
            if parse_embedded_binary_identity(&bytes, identity.target(), identity.release_tag())?
                != identity
            {
                return Err(BundleAcquisitionError::Binary(
                    "Debian binary identity disagrees with release".into(),
                ));
            }
            binaries.insert(path);
        }
    }
    if binaries.len() != 2 {
        return Err(archive_error(
            "Debian package must contain both release binaries",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_bundle::tests::{
        embedded_identity_binary, release_asset, release_info, stable_identity, temp_dir,
        FakeSource,
    };

    fn payload(expected: &ReleaseIdentity, extra: Option<(&str, EntryType, u32)>) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut append = |path: &str, kind, mode, bytes: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_mode(mode);
            header.set_uid(0);
            header.set_gid(0);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, path, Cursor::new(bytes))
                .unwrap();
        };
        append(
            "./usr/bin/lg-buddy",
            EntryType::Regular,
            0o755,
            &embedded_identity_binary(expected),
        );
        let gui = ReleaseIdentity {
            target: GUI_TARGET.into(),
            ..expected.clone()
        };
        append(
            "./usr/bin/lg-buddy-gui",
            EntryType::Regular,
            0o755,
            &embedded_identity_binary(&gui),
        );
        if let Some((path, kind, mode)) = extra {
            append(path, kind, mode, &[]);
        }
        builder.into_inner().unwrap()
    }

    fn source(bytes: &[u8]) -> FakeSource {
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

    fn native_fixture(
        directory: &Path,
        package: &str,
        architecture: &str,
        gui_commit: &str,
    ) -> Option<Vec<u8>> {
        if !Path::new("/usr/bin/dpkg-deb").is_file() {
            eprintln!("native Debian decoding is exercised on Ubuntu CI");
            return None;
        }
        let root = directory.join("fixture");
        fs::create_dir_all(root.join("DEBIAN")).unwrap();
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        let expected = stable_identity();
        fs::write(root.join("DEBIAN/control"), format!("Package: {package}\nVersion: {}\nArchitecture: {architecture}\nMaintainer: LG Buddy <7261268+Staphylococcus@users.noreply.github.com>\nDescription: native decoder fixture\n", native_version(expected.version()).unwrap())).unwrap();
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
            let path = root.join("usr/bin").join(name);
            fs::write(&path, embedded_identity_binary(&identity)).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let output = directory.join("fixture.deb");
        assert!(Command::new("/usr/bin/dpkg-deb")
            .args(["--root-owner-group", "--build"])
            .arg(&root)
            .arg(&output)
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success());
        Some(fs::read(output).unwrap())
    }

    #[test]
    fn version_mapping_agrees_with_the_package_builder_release_grammar() {
        for (upstream, native) in [
            ("2.0.0", "2.0.0-1"),
            ("2.0.0-beta.2.ci", "2.0.0~beta.2.ci-1"),
            ("2.0.0-rc.10", "2.0.0~rc.10-1"),
        ] {
            assert_eq!(
                native_version(&Version::parse(upstream).unwrap()).unwrap(),
                native
            );
        }
        for unsupported in [
            "2.0.0-alpha.a.1",
            "2.0.0-alpha.aa",
            "2.0.0-alpha10",
            "2.0.0-beta.1.2",
            "2.0.0+build",
        ] {
            assert!(native_version(&Version::parse(unsupported).unwrap()).is_err());
        }
    }

    #[test]
    fn package_resolution_requires_exact_debian_assets_without_bundle_assets() {
        let source = source(b"package");
        assert_eq!(
            resolve_with(&release_info(vec![]), &source)
                .unwrap()
                .identity,
            stable_identity()
        );
        let mut missing = source;
        missing.fresh_release = release_info(vec![]);
        assert!(matches!(
            resolve_with(&release_info(vec![]), &missing),
            Err(BundleAcquisitionError::ReleaseMetadata(_))
        ));
    }

    #[test]
    fn data_payload_requires_both_release_identities_and_no_dynamic_paths_or_links() {
        let identity = stable_identity();
        verify_data(&payload(&identity, None), &identity).unwrap();
        for extra in [
            ("etc/lg-buddy/config-path", EntryType::Regular, 0o644),
            ("usr/lib/lg-buddy/config-path", EntryType::Regular, 0o644),
            ("usr/lib/lg-buddy/redirect", EntryType::Symlink, 0o755),
            ("usr/lib/lg-buddy/setuid", EntryType::Regular, 0o4755),
            ("usr/bin/lg-buddy", EntryType::Regular, 0o755),
        ] {
            assert!(verify_data(&payload(&identity, Some(extra)), &identity).is_err());
        }
        let wrong = ReleaseIdentity {
            commit: "a".repeat(40),
            ..identity.clone()
        };
        assert!(verify_data(&payload(&wrong, None), &identity).is_err());
        assert!(verify_data(&[0u8; 1024], &identity).is_err());
    }

    #[test]
    fn control_metadata_cannot_claim_dynamic_configuration_or_duplicate_fields() {
        for (name, content, duplicate) in [
            ("control", b"Package: lg-buddy\n".as_slice(), false),
            (
                "conffiles",
                b"/etc/lg-buddy/config-path\n".as_slice(),
                false,
            ),
            ("control", b"Package: lg-buddy\n".as_slice(), true),
        ] {
            let mut builder = tar::Builder::new(Vec::new());
            for _ in 0..if duplicate { 2 } else { 1 } {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, name, Cursor::new(content))
                    .unwrap();
            }
            assert_eq!(
                verify_control(&builder.into_inner().unwrap()).is_ok(),
                name == "control" && !duplicate
            );
        }
    }

    #[test]
    fn real_debian_acquisition_and_privileged_receipt_verify_the_same_bytes() {
        let directory = temp_dir("debian-acquisition");
        let Some(bytes) =
            native_fixture(&directory, "lg-buddy", "amd64", stable_identity().commit())
        else {
            fs::remove_dir_all(directory).unwrap();
            return;
        };
        let source = source(&bytes);
        let acquired = acquire_with(
            &release_info(vec![]),
            &directory.join("user-cache"),
            &source,
        )
        .unwrap();
        assert_eq!(acquired.identity, stable_identity());
        let received = receive_with(
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
            assert!(receive_with(
                &stable_identity(),
                Cursor::new(bad),
                &directory.join("root-cache"),
                &source
            )
            .is_err());
        }
        drop(acquired);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn native_control_metadata_and_gui_commit_must_match_the_release() {
        let directory = temp_dir("debian-metadata");
        for (name, arch, commit) in [
            ("other-package", "amd64", stable_identity().commit()),
            ("lg-buddy", "arm64", stable_identity().commit()),
            (
                "lg-buddy",
                "amd64",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ] {
            let Some(bytes) = native_fixture(&directory, name, arch, commit) else {
                break;
            };
            let source = source(&bytes);
            assert!(
                acquire_with(&release_info(vec![]), &directory.join("cache"), &source).is_err()
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn receipt_rejects_changed_tag_or_published_checksum_before_accepting_input() {
        let directory = temp_dir("debian-root-authority");
        let mut changed = source(b"package");
        changed.commit = "a".repeat(40);
        assert!(matches!(
            receive_with(
                &stable_identity(),
                Cursor::new(b"package"),
                &directory.join("cache"),
                &changed
            ),
            Err(BundleAcquisitionError::ReleaseMetadata(_))
        ));
        let mut changed = source(b"package");
        let name = asset_name(stable_identity().version()).unwrap();
        let bad_checksum = format!("{}  {name}\n", "a".repeat(64)).into_bytes();
        changed.fresh_release = release_info(vec![
            release_asset(1, &name, b"package"),
            release_asset(2, CHECKSUM_ASSET_NAME, &bad_checksum),
        ]);
        changed.payloads.insert(2, bad_checksum);
        assert!(matches!(
            receive_with(
                &stable_identity(),
                Cursor::new(b"package"),
                &directory.join("cache"),
                &changed
            ),
            Err(BundleAcquisitionError::Checksum(_))
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
