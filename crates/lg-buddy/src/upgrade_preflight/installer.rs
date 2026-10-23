//! Read-only mutation guard for bundle installation and host-integration artifacts.
//! Callers select paths; this module reuses host ownership and mount facts.
use super::{FilesystemFacts, OsFilesystemFacts, PackageDatabase, PathKind, PathOwnership};
use std::collections::HashSet;
use std::io;
use std::path::{Component, Path, PathBuf};

fn refused(path: &Path, reason: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!(
        "installer preflight refused: {}: {reason}",
        path.display()
    ))
}

fn absolute(path: &Path) -> io::Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        || path.components().collect::<PathBuf>().as_os_str() != path.as_os_str()
    {
        return Err(refused(path, "expected a canonical absolute path"));
    }
    Ok(())
}

struct Guard<'a, F> {
    filesystem: &'a F,
    packages: &'a PackageDatabase,
    root: &'a Path,
    owners_checked: HashSet<PathBuf>,
}

impl<F: FilesystemFacts> Guard<'_, F> {
    fn owner(&mut self, path: &Path) -> io::Result<()> {
        if self.owners_checked.contains(path) {
            return Ok(());
        }
        match self
            .packages
            .owner_of(path)
            .map_err(|error| refused(path, error))?
        {
            PathOwnership::Unowned => {}
            PathOwnership::Owned(owner) => {
                return Err(refused(
                    path,
                    format!(
                        "owned by {} package {}; use the system package manager",
                        owner.family.as_str(),
                        owner.name
                    ),
                ));
            }
            PathOwnership::Conflicting(owners) => {
                let names = owners
                    .iter()
                    .map(|owner| format!("{}:{}", owner.family.as_str(), owner.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(refused(
                    path,
                    format!("conflicting package ownership: {names}"),
                ));
            }
        }
        self.owners_checked.insert(path.to_owned());
        Ok(())
    }

    // Missing parents will be created by mkdir -p, so those are destinations
    // too. Existing shared directories are only traversed, never chmodded.
    fn parents(&mut self, path: &Path) -> io::Result<()> {
        absolute(path)?;
        if path == self.root || !path.starts_with(self.root) {
            return Err(refused(
                path,
                "destination must be below the installation root",
            ));
        }
        let mut nearest = true;
        for parent in path.ancestors().skip(1) {
            match self.filesystem.path_facts(parent) {
                Ok(facts) => {
                    if facts.kind != PathKind::Directory {
                        return Err(refused(
                            parent,
                            "destination ancestor is not a directory or is a symlink",
                        ));
                    }
                    // A writable mount may sit below a read-only ancestor.
                    if nearest && facts.read_only_filesystem {
                        return Err(refused(
                            parent,
                            "destination filesystem is mounted read-only",
                        ));
                    }
                    nearest = false;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => self.owner(parent)?,
                Err(error) => return Err(refused(parent, error)),
            }
        }
        Ok(())
    }

    fn destination(&mut self, path: &Path, directory: bool, removing: bool) -> io::Result<()> {
        self.parents(path)?;
        match self.filesystem.path_facts(path) {
            Ok(facts) => {
                if (directory && facts.kind != PathKind::Directory)
                    || (!removing
                        && facts.kind
                            != if directory {
                                PathKind::Directory
                            } else {
                                PathKind::File
                            })
                {
                    return Err(refused(
                        path,
                        "destination has an unexpected type or is a symlink",
                    ));
                }
                if facts.read_only_filesystem {
                    return Err(refused(path, "destination filesystem is mounted read-only"));
                }
                if facts.mount_point && (!directory || removing) {
                    return Err(refused(path, "refusing to replace or remove a mount point"));
                }
                if directory && !removing {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(refused(path, error)),
        }
        self.owner(path)
    }

    fn remove_tree(&mut self, path: &Path) -> io::Result<()> {
        self.destination(path, false, true)?;
        match self.filesystem.path_facts(path) {
            Ok(facts) if facts.kind == PathKind::Directory => {
                for entry in self
                    .filesystem
                    .read_directory(path)
                    .map_err(|error| refused(path, error))?
                {
                    self.remove_tree(&entry)?;
                }
            }
            Ok(_) => {} // rm -rf unlinks symlinks; it does not follow them.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(refused(path, error)),
        }
        Ok(())
    }

    fn copy_tree(&mut self, source: &Path, destination: &Path) -> io::Result<()> {
        self.destination(destination, true, false)?;
        for entry in self
            .filesystem
            .read_directory(source)
            .map_err(|error| refused(source, error))?
        {
            let target = destination.join(
                entry
                    .file_name()
                    .ok_or_else(|| refused(&entry, "missing filename"))?,
            );
            match self
                .filesystem
                .path_facts(&entry)
                .map_err(|error| refused(&entry, error))?
                .kind
            {
                PathKind::Directory => self.copy_tree(&entry, &target)?,
                PathKind::File => self.destination(&target, false, false)?,
                _ => {
                    return Err(refused(
                        &entry,
                        "payload tree contains a symlink or special file",
                    ))
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn mutable_host(filesystem: &impl FilesystemFacts, root: &Path) -> io::Result<()> {
    absolute(root)?;
    for marker in ["etc/NIXOS", "run/ostree-booted"] {
        let marker = root.join(marker);
        match filesystem.path_facts(&marker) {
            Ok(_) => {
                return Err(refused(
                    &marker,
                    "immutable/declarative host; manage LG Buddy through the host configuration",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(refused(&marker, error)),
        }
    }
    Ok(())
}

/// KWin's host-required artifact exception: guard the individual plugin file.
/// Existing vendor-owned container directories are traversed without mutation.
/// The caller still constrains the loader root and application-specific filename.
pub(crate) fn check_host_artifact(
    filesystem: &impl FilesystemFacts,
    packages: &PackageDatabase,
    root: &Path,
    destination: &Path,
    removing: bool,
) -> io::Result<()> {
    mutable_host(filesystem, root)?;
    Guard {
        filesystem,
        packages,
        root,
        owners_checked: HashSet::new(),
    }
    .destination(destination, false, removing)
}

fn check(
    filesystem: &impl FilesystemFacts,
    packages: &PackageDatabase,
    arguments: &[String],
) -> io::Result<()> {
    let Some((root, mut operations)) = arguments.split_first() else {
        return Err(io::Error::other(
            "installer preflight requires an installation root and mutation plan",
        ));
    };
    let root = Path::new(root);
    mutable_host(filesystem, root)?;
    let mut guard = Guard {
        filesystem,
        packages,
        root,
        owners_checked: HashSet::new(),
    };
    if operations.is_empty() {
        return Err(io::Error::other(
            "installer preflight requires a nonempty mutation plan",
        ));
    }
    while let Some((operation, rest)) = operations.split_first() {
        let Some((path, rest)) = rest.split_first() else {
            return Err(io::Error::other("incomplete installer mutation plan"));
        };
        let path = Path::new(path);
        operations = rest;
        match operation.as_str() {
            "file" => guard.destination(path, false, false)?,
            "mkdir" => guard.destination(path, true, false)?,
            "rmdir" | "directory" => guard.destination(path, true, true)?,
            "link" => {
                guard.destination(path, false, true)?;
                if filesystem
                    .path_facts(path)
                    .is_ok_and(|facts| facts.kind != PathKind::Symlink)
                {
                    return Err(refused(path, "expected a systemd enable symlink"));
                }
            }
            "remove" => {
                guard.destination(path, false, true)?;
                if filesystem
                    .path_facts(path)
                    .is_ok_and(|facts| facts.kind == PathKind::Directory)
                {
                    return Err(refused(path, "expected a file to remove"));
                }
            }
            "tree" => guard.remove_tree(path)?,
            "copy-tree" => {
                let Some((destination, rest)) = operations.split_first() else {
                    return Err(io::Error::other("missing copy-tree destination"));
                };
                guard.copy_tree(path, Path::new(destination))?;
                operations = rest;
            }
            _ => return Err(io::Error::other("unknown installer mutation operation")),
        }
    }
    Ok(())
}

/// Private installer protocol. No config loading, service changes, or writes.
pub fn run(arguments: &[String]) -> io::Result<()> {
    check(&OsFilesystemFacts, &PackageDatabase::system(), arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade_preflight::{Database, PackageFamily, PathFacts, ProbeOutcome};
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "lg-buddy-installer-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn plan(&self, operations: &[(&str, &str)]) -> Vec<String> {
            let mut result = vec![self.0.display().to_string()];
            for (operation, path) in operations {
                result.extend([
                    operation.to_string(),
                    self.0.join(path).display().to_string(),
                ]);
            }
            result
        }
        fn file(&self, path: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "retained").unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn packages(target: PathBuf, result: ProbeOutcome) -> PackageDatabase {
        PackageDatabase::with_probe(
            vec![Database {
                name: "dpkg-query",
                exec: "/usr/bin/dpkg-query",
                args: &["-S"],
                family: PackageFamily::Dpkg,
                env_remove: &[],
            }],
            move |_, path| {
                if path == target {
                    result.clone()
                } else {
                    ProbeOutcome::Ran {
                        code: 1,
                        stdout: String::new(),
                        stderr: String::new(),
                    }
                }
            },
        )
    }
    fn owned(path: &Path) -> ProbeOutcome {
        ProbeOutcome::Ran {
            code: 0,
            stdout: format!("host-package: {}\n", path.display()),
            stderr: String::new(),
        }
    }
    fn unowned() -> PackageDatabase {
        packages(PathBuf::new(), ProbeOutcome::Unavailable)
    }

    struct ReadOnlyAt(PathBuf);
    impl FilesystemFacts for ReadOnlyAt {
        fn path_facts(&self, path: &Path) -> io::Result<PathFacts> {
            let mut facts = OsFilesystemFacts.path_facts(path)?;
            facts.read_only_filesystem |= path.starts_with(&self.0);
            Ok(facts)
        }
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            OsFilesystemFacts.read_to_string(path)
        }
        fn read_directory(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            OsFilesystemFacts.read_directory(path)
        }
        fn mount_points(&self) -> io::Result<Vec<PathBuf>> {
            OsFilesystemFacts.mount_points()
        }
    }

    #[test]
    fn late_owned_destination_refuses_before_changing_anything() {
        let f = Fixture::new();
        let last = f.file("usr/share/application.desktop");
        let plan = f.plan(&[
            ("file", "etc/lg-buddy/config-path"),
            ("file", "usr/bin/lg-buddy"),
            ("file", "usr/share/application.desktop"),
        ]);
        let error = check(
            &OsFilesystemFacts,
            &packages(last.clone(), owned(&last)),
            &plan,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("host-package"));
        assert!(error.contains(last.to_str().unwrap()));
        assert_eq!(fs::read_to_string(last).unwrap(), "retained");
        assert!(!f.0.join("etc").exists());
        assert!(!f.0.join("usr/bin").exists());
    }

    #[test]
    fn conflicting_unknown_and_failed_queries_all_refuse() {
        let f = Fixture::new();
        let path = f.0.join("usr/bin/lg-buddy");
        for (outcome, expected) in [
            (ProbeOutcome::Unavailable, "no package database"),
            (
                ProbeOutcome::Failed("injected database failure".into()),
                "probe failed",
            ),
            (
                ProbeOutcome::Ran {
                    code: 0,
                    stdout: format!("one: {}\ntwo: {}\n", path.display(), path.display()),
                    stderr: String::new(),
                },
                "conflicting package ownership",
            ),
            (
                ProbeOutcome::Ran {
                    code: 0,
                    stdout: "malformed".into(),
                    stderr: String::new(),
                },
                "probe failed",
            ),
        ] {
            // Ensure the target, rather than a missing ancestor, is queried.
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let error = check(
                &OsFilesystemFacts,
                &packages(path.clone(), outcome),
                &f.plan(&[("file", "usr/bin/lg-buddy")]),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!path.exists());
        }
    }

    #[test]
    fn new_files_use_mount_facts_without_ordinary_user_writability_tests() {
        let f = Fixture::new();
        let directory = f.0.join("usr");
        fs::create_dir(&directory).unwrap();
        let plan = f.plan(&[("file", "usr/bin/lg-buddy")]);
        let error = check(&ReadOnlyAt(directory.clone()), &unowned(), &plan)
            .unwrap_err()
            .to_string();
        assert!(error.contains("mounted read-only"), "{error}");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
        let result = check(&OsFilesystemFacts, &unowned(), &plan);
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        result.unwrap();
    }

    #[test]
    fn reused_shared_directories_are_not_package_mutations_but_missing_ones_are() {
        let f = Fixture::new();
        let shared = f.0.join("usr/bin");
        fs::create_dir_all(&shared).unwrap();
        check(
            &OsFilesystemFacts,
            &packages(shared.clone(), owned(&shared)),
            &f.plan(&[("mkdir", "usr/bin"), ("file", "usr/bin/lg-buddy")]),
        )
        .unwrap();
        fs::remove_dir(&shared).unwrap();
        assert!(check(
            &OsFilesystemFacts,
            &packages(shared.clone(), owned(&shared)),
            &f.plan(&[("file", "usr/bin/lg-buddy")])
        )
        .is_err());
    }

    #[test]
    fn recursive_removals_check_descendants_without_following_symlinks() {
        let f = Fixture::new();
        let nested = f.file("usr/lib/lg-buddy/kwin/old/plugin.so");
        let plan = f.plan(&[("tree", "usr/lib/lg-buddy/kwin")]);
        assert!(check(
            &OsFilesystemFacts,
            &packages(nested.clone(), owned(&nested)),
            &plan
        )
        .is_err());
        let external = f.file("external/config.env");
        symlink(&external, f.0.join("usr/lib/lg-buddy/kwin/python-link")).unwrap();
        check(
            &OsFilesystemFacts,
            &packages(external.clone(), owned(&external)),
            &plan,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(external).unwrap(), "retained");
    }

    #[test]
    fn new_copy_tree_members_are_checked_even_when_not_on_disk() {
        let f = Fixture::new();
        f.file("payload/prebuilt/plugin.so");
        let ghost = f.0.join("usr/lib/lg-buddy/kwin/prebuilt/plugin.so");
        let mut plan = f.plan(&[("tree", "usr/lib/lg-buddy/kwin"), ("copy-tree", "payload")]);
        plan.push(f.0.join("usr/lib/lg-buddy/kwin").display().to_string());
        assert!(check(
            &OsFilesystemFacts,
            &packages(ghost.clone(), owned(&ghost)),
            &plan
        )
        .is_err());
        check(&OsFilesystemFacts, &unowned(), &plan).unwrap();
        assert!(!ghost.exists());
    }

    #[test]
    fn redirected_parent_and_immutable_host_are_refused() {
        let f = Fixture::new();
        fs::create_dir(f.0.join("external")).unwrap();
        symlink(f.0.join("external"), f.0.join("usr")).unwrap();
        assert!(check(
            &OsFilesystemFacts,
            &unowned(),
            &f.plan(&[("file", "usr/bin/lg-buddy")])
        )
        .unwrap_err()
        .to_string()
        .contains("symlink"));
        f.file("run/ostree-booted");
        assert!(check(
            &OsFilesystemFacts,
            &unowned(),
            &f.plan(&[("file", "etc/config")])
        )
        .unwrap_err()
        .to_string()
        .contains("immutable/declarative"));
    }

    #[test]
    fn host_artifact_exception_preserves_vendor_directories_and_checks_each_file() {
        let f = Fixture::new();
        let artifact = f.file("usr/lib/qt6/plugins/kwin/plugins/plugin.so");
        let directory = artifact.parent().unwrap();
        let before = fs::metadata(directory).unwrap().permissions().mode();
        // KWin itself can own the shared directory; only the artifact is a
        // mutation destination. Traversing the container does not adopt it.
        for removing in [false, true] {
            check_host_artifact(
                &OsFilesystemFacts,
                &packages(directory.to_owned(), owned(directory)),
                &f.0,
                &artifact,
                removing,
            )
            .unwrap();
            for result in [
                owned(&artifact),
                ProbeOutcome::Ran {
                    code: 0,
                    stdout: format!(
                        "first: {}\nsecond: {}\n",
                        artifact.display(),
                        artifact.display()
                    ),
                    stderr: String::new(),
                },
                ProbeOutcome::Failed("database unavailable".into()),
                ProbeOutcome::Unavailable,
            ] {
                assert!(check_host_artifact(
                    &OsFilesystemFacts,
                    &packages(artifact.clone(), result),
                    &f.0,
                    &artifact,
                    removing,
                )
                .is_err());
            }
        }
        assert_eq!(fs::read_to_string(&artifact).unwrap(), "retained");
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode(),
            before
        );
    }

    #[test]
    fn host_artifact_exception_refuses_read_only_filesystems_and_managed_hosts() {
        for existing in [false, true] {
            let f = Fixture::new();
            let directory = f.0.join("usr/lib/qt6/plugins/kwin/plugins");
            fs::create_dir_all(&directory).unwrap();
            let artifact = directory.join("plugin.so");
            if existing {
                fs::write(&artifact, "retained").unwrap();
            }
            for removing in [false, true] {
                assert!(check_host_artifact(
                    &ReadOnlyAt(directory.clone()),
                    &unowned(),
                    &f.0,
                    &artifact,
                    removing,
                )
                .unwrap_err()
                .to_string()
                .contains("read-only"));
            }
        }
        for marker in ["etc/NIXOS", "run/ostree-booted"] {
            let f = Fixture::new();
            let artifact = f.file("usr/lib/qt6/plugins/kwin/plugins/plugin.so");
            f.file(marker);
            for removing in [false, true] {
                assert!(check_host_artifact(
                    &OsFilesystemFacts,
                    &unowned(),
                    &f.0,
                    &artifact,
                    removing,
                )
                .unwrap_err()
                .to_string()
                .contains("immutable/declarative"));
            }
        }
    }

    #[test]
    fn malformed_or_escaping_plans_cannot_pass() {
        let f = Fixture::new();
        for plan in [
            vec![],
            vec![f.0.display().to_string()],
            f.plan(&[("unknown", "usr/file")]),
            f.plan(&[("file", "../escape")]),
            vec![f.0.display().to_string(), "file".into(), "/outside".into()],
        ] {
            assert!(check(&OsFilesystemFacts, &unowned(), &plan).is_err());
        }
    }
}
