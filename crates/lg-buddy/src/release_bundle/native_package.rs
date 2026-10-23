//! Shared native-package acquisition and bounded, non-installing decoding.
use super::*;
use std::io::Cursor;
use std::process::{Command, Stdio};

#[derive(Clone, Copy)]
pub(super) enum NativeFormat {
    Debian,
    Rpm,
}
impl NativeFormat {
    fn asset_name(self, version: &Version) -> Result<String, BundleAcquisitionError> {
        match self {
            Self::Debian => debian::asset_name(version),
            Self::Rpm => rpm::asset_name(version),
        }
    }
    fn verify_package(
        self,
        file: &File,
        expected: &ReleaseIdentity,
    ) -> Result<(), BundleAcquisitionError> {
        match self {
            Self::Debian => debian::verify_package(file, expected),
            Self::Rpm => rpm::verify_package(file, expected),
        }
    }
}
#[derive(Debug)]
pub(crate) struct VerifiedNativePackage {
    pub(crate) identity: ReleaseIdentity,
    file: File,
    _staging: StagingDirectory,
}

impl VerifiedNativePackage {
    pub(crate) fn input(&self) -> Result<File, BundleAcquisitionError> {
        rewind_clone(&self.file)
    }
}

pub(crate) struct RootNativePackage {
    pub(crate) path: PathBuf,
    _staging: StagingDirectory,
}

pub(super) fn resolve_with<S: GitHubSource>(
    format: NativeFormat,
    release: &ReleaseInfo,
    source: &S,
) -> Result<ResolvedRelease, BundleAcquisitionError> {
    let release = fresh_release_with(release, source)?;
    select_named_release_assets(&release, &format.asset_name(release.version())?)?;
    let identity = resolve_tag_identity(&release, source)?;
    Ok(ResolvedRelease { release, identity })
}

pub(super) fn resolve_identity(
    format: NativeFormat,
    release: &ReleaseInfo,
) -> Result<ReleaseIdentity, BundleAcquisitionError> {
    resolve_with(format, release, &UreqGitHubSource::new()).map(|resolved| resolved.identity)
}

pub(super) fn acquire(
    format: NativeFormat,
    release: &ReleaseInfo,
) -> Result<VerifiedNativePackage, BundleAcquisitionError> {
    acquire_with(
        format,
        release,
        &acquisition_cache_root_from_env()?,
        &UreqGitHubSource::new(),
    )
}

pub(super) fn acquire_with<S: GitHubSource>(
    format: NativeFormat,
    release: &ReleaseInfo,
    cache: &Path,
    source: &S,
) -> Result<VerifiedNativePackage, BundleAcquisitionError> {
    let staging = StagingDirectory::create(cache)?;
    let resolved = resolve_with(format, release, source)?;
    let selected = select_named_release_assets(
        &resolved.release,
        &format.asset_name(resolved.release.version())?,
    )?;
    verify_published_digest(source, &staging, &selected)?;
    let path = staging.path.join(selected.archive.name());
    let downloaded = source.download_asset(selected.archive, &path, MAX_ARCHIVE_BYTES)?;
    pin_download_file(&path, &downloaded.file)?;
    verify_download(selected.archive, &downloaded.facts)?;
    format.verify_package(&downloaded.file, &resolved.identity)?;
    Ok(VerifiedNativePackage {
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
            "Native package and published checksum disagree".into(),
        ));
    }
    Ok(())
}

/// Reobserve the official release under privilege, then receive only the exact
/// published bytes into a root-owned staging directory. No caller file path or
/// caller-provided digest grants installation authority.
pub(super) fn receive(
    format: NativeFormat,
    expected: &ReleaseIdentity,
    input: impl Read,
    cache: &Path,
) -> Result<RootNativePackage, BundleAcquisitionError> {
    receive_with(format, expected, input, cache, &UreqGitHubSource::new())
}

pub(super) fn receive_with<S: GitHubSource>(
    format: NativeFormat,
    expected: &ReleaseIdentity,
    input: impl Read,
    cache: &Path,
    source: &S,
) -> Result<RootNativePackage, BundleAcquisitionError> {
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
    let resolved = resolve_with(format, &selected_release, source)?;
    if resolved.identity != *expected {
        return Err(BundleAcquisitionError::ReleaseMetadata(
            "Native release changed after confirmation".into(),
        ));
    }
    let selected =
        select_named_release_assets(&resolved.release, &format.asset_name(expected.version())?)?;
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
    format.verify_package(&file, expected)?;
    Ok(RootNativePackage {
        path,
        _staging: staging,
    })
}

pub(super) fn decode(
    file: &File,
    program: &str,
    arguments: &[&str],
    limit: u64,
) -> Result<Vec<u8>, BundleAcquisitionError> {
    let mut child = Command::new(program)
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
            "native decoder failed or exceeded the payload limit".into(),
        ));
    }
    Ok(bytes)
}

pub(super) fn verify_data(
    data: &[u8],
    expected: &ReleaseIdentity,
) -> Result<HashSet<String>, BundleAcquisitionError> {
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
                "Native payload has unsafe types, owners or modes",
            ));
        }
        let raw = entry.path_bytes();
        let raw = raw.strip_prefix(b"./").unwrap_or(&raw);
        if raw.is_empty() || raw == b"." {
            if kind == EntryType::Directory {
                continue;
            }
            return Err(archive_error("Native root entry is not a directory"));
        }
        let path = normalize_archive_path(raw, kind)?;
        if seen.len() >= MAX_ARCHIVE_ENTRIES
            || !seen.insert(path.clone())
            || !(path == "usr" || path.starts_with("usr/"))
            || path == "usr/lib/lg-buddy/config-path"
            || entry.size() > MAX_ARCHIVE_FILE_BYTES
        {
            return Err(archive_error(format!(
                "unsafe Native payload entry: {path}"
            )));
        }
        if path == "usr/bin/lg-buddy" || path == "usr/bin/lg-buddy-gui" {
            if kind != EntryType::Regular || entry.header().mode().map_err(archive_error)? != 0o755
            {
                return Err(archive_error("Native binary must be a regular executable"));
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
                    "Native binary identity disagrees with release".into(),
                ));
            }
            binaries.insert(path);
        }
    }
    if binaries.len() != 2 {
        return Err(archive_error(
            "Native package must contain both release binaries",
        ));
    }
    Ok(seen)
}
