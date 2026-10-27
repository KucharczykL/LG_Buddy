//! Observe the running payload, rather than an OS marker or caller assertion.
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::InstallationOwnership;

pub(crate) fn running_nix_ownership() -> Option<InstallationOwnership> {
    match std::env::current_exe() {
        Ok(executable) => observe(
            &executable,
            |path| path.canonicalize(),
            |path| {
                let metadata = std::fs::symlink_metadata(path)?;
                Ok((
                    metadata.is_dir(),
                    metadata.is_file(),
                    metadata.uid(),
                    metadata.mode(),
                ))
            },
        ),
        Err(_) => Some(InstallationOwnership::Unknown),
    }
}

fn observe(
    executable: &Path,
    resolve: impl Fn(&Path) -> io::Result<PathBuf>,
    facts: impl Fn(&Path) -> io::Result<(bool, bool, u32, u32)>,
) -> Option<InstallationOwnership> {
    let Ok(executable) = resolve(executable) else {
        return Some(InstallationOwnership::Unknown);
    };
    let Ok(relative) = executable.strip_prefix("/nix/store") else {
        return None;
    };
    let parts: Vec<_> = relative.components().collect();
    let valid = || -> Option<()> {
        if parts.len() != 3 || parts[1].as_os_str() != "bin" {
            return None;
        }
        let name = parts[0].as_os_str().to_str()?;
        let (hash, suffix) = name.split_once('-')?;
        if hash.len() != 32
            || suffix.is_empty()
            || !hash
                .bytes()
                .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b))
            || !matches!(
                parts[2].as_os_str().to_str()?,
                "lg-buddy" | "lg-buddy-gui" | ".lg-buddy-gui-wrapped"
            )
        {
            return None;
        }
        // Store entries and their payload cannot be writable or redirected.
        // The daemon's store directory itself may be sticky and group-writable.
        for directory in [
            PathBuf::from("/nix"),
            PathBuf::from("/nix/store"),
            Path::new("/nix/store").join(name),
            executable.parent()?.to_owned(),
        ] {
            let (directory_kind, _, owner, mode) = facts(&directory).ok()?;
            if !directory_kind || owner != 0 {
                return None;
            }
            if directory == Path::new("/nix/store") {
                if mode & 0o022 != 0 && mode & 0o1000 == 0 {
                    return None;
                }
            } else if directory == Path::new("/nix") {
                if mode & 0o022 != 0 {
                    return None;
                }
            } else if mode & 0o222 != 0 {
                return None;
            }
        }
        for file in [executable.clone(), executable.with_file_name("lg-buddy")] {
            let (_, regular, owner, mode) = facts(&file).ok()?;
            if !regular || owner != 0 || mode & 0o222 != 0 || mode & 0o111 == 0 {
                return None;
            }
        }
        Some(())
    };
    Some(if valid().is_some() {
        InstallationOwnership::NixStore
    } else {
        InstallationOwnership::Unknown
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE: &str = "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-lg-buddy-2.0.0";

    fn facts(path: &Path) -> io::Result<(bool, bool, u32, u32)> {
        let directory = !matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("lg-buddy" | "lg-buddy-gui" | ".lg-buddy-gui-wrapped")
        );
        Ok((
            directory,
            !directory,
            0,
            if path == Path::new("/nix/store") {
                0o1775
            } else {
                0o555
            },
        ))
    }

    #[test]
    fn canonical_payload_and_profile_aliases_use_the_same_store_owner() {
        for binary in ["lg-buddy", "lg-buddy-gui", ".lg-buddy-gui-wrapped"] {
            let canonical = PathBuf::from(format!("{PACKAGE}/bin/{binary}"));
            for alias in [
                &canonical,
                Path::new("/home/user/.nix-profile/bin/lg-buddy"),
                Path::new("/run/current-system/sw/bin/lg-buddy"),
            ] {
                assert_eq!(
                    observe(alias, |_| Ok(canonical.clone()), facts),
                    Some(InstallationOwnership::NixStore)
                );
            }
        }
    }

    #[test]
    fn os_names_or_store_lookalikes_do_not_select_nix_ownership() {
        for path in [
            "/usr/bin/lg-buddy",
            "/var/lib/lg-buddy/bin/lg-buddy",
            "/tmp/nix/store/package/bin/lg-buddy",
        ] {
            assert_eq!(
                observe(
                    Path::new(path),
                    |p| Ok(p.into()),
                    |_| panic!("not a store payload")
                ),
                None
            );
        }
        for path in [
            "/nix/store/not-a-store-hash/bin/lg-buddy",
            "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-package/bin/other",
        ] {
            assert_eq!(
                observe(Path::new(path), |p| Ok(p.into()), facts),
                Some(InstallationOwnership::Unknown)
            );
        }
    }

    #[test]
    fn writable_foreign_missing_or_redirected_payloads_fail_closed() {
        let executable = PathBuf::from(format!("{PACKAGE}/bin/lg-buddy"));
        for bad_path in [
            PathBuf::from("/nix"),
            PathBuf::from("/nix/store"),
            PathBuf::from(PACKAGE),
            executable.parent().unwrap().into(),
            executable.clone(),
        ] {
            for bad in [
                (false, false, 0, 0o555),
                (true, true, 1000, 0o555),
                (true, true, 0, 0o777),
            ] {
                assert_eq!(
                    observe(
                        &executable,
                        |p| Ok(p.into()),
                        |p| if p == bad_path { Ok(bad) } else { facts(p) }
                    ),
                    Some(InstallationOwnership::Unknown)
                );
            }
            assert_eq!(
                observe(
                    &executable,
                    |p| Ok(p.into()),
                    |p| if p == bad_path {
                        Err(io::ErrorKind::PermissionDenied.into())
                    } else {
                        facts(p)
                    }
                ),
                Some(InstallationOwnership::Unknown)
            );
        }
        assert_eq!(
            observe(&executable, |_| Err(io::ErrorKind::NotFound.into()), facts),
            Some(InstallationOwnership::Unknown)
        );
    }
}
