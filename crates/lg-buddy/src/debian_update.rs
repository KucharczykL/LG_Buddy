//! APT owns the native package transaction. The privileged entrypoint accepts
//! a release identity and package bytes, never a caller-selected path or command.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use semver::Version;

use crate::release_bundle::{
    debian, verify_release_binary_identity, verify_release_gui_binary_identity, ReleaseIdentity,
};
use crate::updates::UpdateChannel;
use crate::upgrade_preflight::{
    FilesystemFacts, OsFilesystemFacts, PackageDatabase, PackageFamily, PathKind, PathOwnership,
};

const RUNTIME: &str = "/usr/bin/lg-buddy";
const GUI: &str = "/usr/bin/lg-buddy-gui";

fn native_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .env("HOME", "/root")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::null());
    command
}

fn capture(command: Command) -> Result<String, String> {
    let output =
        crate::update_install::run_bounded_command(command).map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(output.diagnostic());
    }
    Ok(output.stdout)
}

fn verify_owners() -> Result<(), String> {
    verify_native_paths(&OsFilesystemFacts)?;
    let database = PackageDatabase::system();
    for path in [RUNTIME, GUI] {
        match database.owner_of(Path::new(path)) {
            Ok(PathOwnership::Owned(owner)) if owner.family == PackageFamily::Dpkg
                && matches!(owner.name.as_str(), "lg-buddy" | "lg-buddy:amd64") => {},
            _ => return Err(format!("APT update requires both binaries to be owned by the lg-buddy Debian package: {path}")),
        }
    }
    Ok(())
}

fn verify_native_paths(filesystem: &impl FilesystemFacts) -> Result<(), String> {
    crate::upgrade_preflight::installer::mutable_host(filesystem, Path::new("/"))
        .map_err(|error| error.to_string())?;
    for binary in [RUNTIME, GUI] {
        for path in Path::new(binary).ancestors() {
            let facts = filesystem
                .path_facts(path)
                .map_err(|error| error.to_string())?;
            let executable = path == Path::new(binary);
            if facts.owner_uid != 0
                || facts.mode & 0o022 != 0
                || facts.read_only_filesystem
                || (executable
                    && (facts.kind != PathKind::File
                        || facts.link_count != 1
                        || facts.mode & 0o7777 != 0o755
                        || facts.mount_point))
                || (!executable && facts.kind != PathKind::Directory)
            {
                return Err(format!(
                    "Native package path is unsafe or read-only: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn installed_version() -> Result<String, String> {
    verify_owners()?;
    for program in ["/usr/bin/apt-get", "/usr/bin/dpkg-deb"] {
        if !Path::new(program).is_file() {
            return Err(format!("Native update tool is unavailable: {program}"));
        }
    }
    let mut query = native_command("/usr/bin/dpkg-query");
    query.args([
        "--admindir=/var/lib/dpkg",
        "--show",
        "--showformat=${Status}\n${Version}\n${Architecture}\n",
        "lg-buddy",
    ]);
    let text = capture(query)?;
    parse_installed_state(&text)
}

fn parse_installed_state(text: &str) -> Result<String, String> {
    let lines: Vec<_> = text.lines().collect();
    match lines.as_slice() {
        ["install ok installed", version, "amd64"] if !version.is_empty() => Ok((*version).to_string()),
        _ => Err("The lg-buddy Debian package must be fully installed for amd64 and unheld; inspect its APT/dpkg state before retrying".into()),
    }
}

pub(crate) fn candidate_preflight(expected: &ReleaseIdentity) -> Result<(), String> {
    let current = installed_version()?;
    let target = debian::native_version(expected.version()).map_err(|error| error.to_string())?;
    let mut compare = native_command("/usr/bin/dpkg");
    compare.args(["--compare-versions", &current, "lt", &target]);
    let output =
        crate::update_install::run_bounded_command(compare).map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Refusing an APT update from {current} to {target}; start a fresh update check"
        ));
    }
    Ok(())
}

pub(crate) fn verify_installed(expected: &ReleaseIdentity) -> Result<(), String> {
    let observed = installed_version()?;
    let target = debian::native_version(expected.version()).map_err(|error| error.to_string())?;
    if observed != target {
        return Err(format!(
            "APT installed version {observed}, expected {target}"
        ));
    }
    verify_release_binary_identity(Path::new(RUNTIME), expected)
        .map_err(|error| error.to_string())?;
    verify_release_gui_binary_identity(Path::new(GUI), expected)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn installer_command(
    package: &debian::VerifiedDebianPackage,
    graphical: bool,
) -> Result<Command, String> {
    let mut command = if unsafe { libc::geteuid() } == 0 {
        Command::new(RUNTIME)
    } else {
        let mut command = Command::new(if graphical || !Path::new("/usr/bin/sudo").is_file() {
            "/usr/bin/pkexec"
        } else {
            "/usr/bin/sudo"
        });
        command.arg(RUNTIME);
        command
    };
    command
        .arg("apply-debian-update")
        .arg(package.identity.release_tag())
        .arg(package.identity.commit())
        .stdin(Stdio::from(
            package.input().map_err(|error| error.to_string())?,
        ));
    Ok(command)
}

fn apt_command(package: &Path, simulate: bool) -> Command {
    let mut command = native_command("/usr/bin/apt-get");
    command.args([
        "--no-remove",
        "--no-install-recommends",
        "--no-allow-downgrades",
        "--no-allow-change-held-packages",
        "--no-allow-unauthenticated",
    ]);
    command.arg(if simulate { "--simulate" } else { "--yes" });
    command.args(["install", "--"]).arg(package);
    command
}

/// Private root-only protocol, called through the ordinary administrator
/// authorization boundary. Fixed paths and a clean child environment keep APT
/// configuration and executables under host administrator control.
pub fn run(arguments: &[String]) -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("Debian package installation requires administrator authorization".into());
    }
    let [tag, commit] = arguments else {
        return Err("Expected a release tag and full commit identity".into());
    };
    let version = tag
        .strip_prefix('v')
        .and_then(|value| Version::parse(value).ok())
        .ok_or("Invalid Debian release tag")?;
    if tag != &format!("v{version}")
        || commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Invalid Debian release identity".into());
    }
    let channel = if version.pre.is_empty() {
        UpdateChannel::Stable
    } else {
        UpdateChannel::Prerelease
    };
    let expected =
        ReleaseIdentity::from_parts(tag, version, channel, "x86_64-unknown-linux-musl", commit);
    candidate_preflight(&expected)?;
    let input: File = io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|error| error.to_string())?
        .into();
    let metadata = input.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("Expected a bounded regular package file on standard input".into());
    }
    let package = debian::receive(
        &expected,
        input,
        &PathBuf::from("/run/lg-buddy-package-updates"),
    )
    .map_err(|error| error.to_string())?;
    capture(apt_command(&package.path, true))?;
    candidate_preflight(&expected)?;
    println!("LG_BUDDY_INSTALL_STATUS=mutation_started");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let status = apt_command(&package.path, false)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("APT transaction failed with status {status}; inspect and repair the package state before retrying"));
    }
    verify_installed(&expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade_preflight::PathFacts;
    use std::collections::HashMap;
    use std::ffi::OsStr;

    struct NativeFacts(HashMap<PathBuf, PathFacts>);

    impl FilesystemFacts for NativeFacts {
        fn path_facts(&self, path: &Path) -> io::Result<PathFacts> {
            self.0
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn read_to_string(&self, _path: &Path) -> io::Result<String> {
            unreachable!()
        }
        fn read_directory(&self, _path: &Path) -> io::Result<Vec<PathBuf>> {
            unreachable!()
        }
        fn mount_points(&self) -> io::Result<Vec<PathBuf>> {
            unreachable!()
        }
    }

    fn native_facts() -> NativeFacts {
        let mut facts = HashMap::new();
        for binary in [RUNTIME, GUI] {
            for path in Path::new(binary).ancestors() {
                facts.insert(
                    path.to_owned(),
                    PathFacts {
                        kind: if path == Path::new(binary) {
                            PathKind::File
                        } else {
                            PathKind::Directory
                        },
                        owner_uid: 0,
                        mode: 0o755,
                        link_count: 1,
                        read_only_filesystem: false,
                        mount_point: false,
                    },
                );
            }
        }
        NativeFacts(facts)
    }

    #[test]
    fn native_updates_keep_the_managed_host_and_filesystem_boundary() {
        verify_native_paths(&native_facts()).unwrap();
        for marker in ["/etc/NIXOS", "/run/ostree-booted"] {
            let mut facts = native_facts();
            facts
                .0
                .insert(marker.into(), facts.0[Path::new(RUNTIME)].clone());
            assert!(verify_native_paths(&facts).is_err());
        }
        let mut facts = native_facts();
        facts
            .0
            .get_mut(Path::new("/usr"))
            .unwrap()
            .read_only_filesystem = true;
        assert!(verify_native_paths(&facts).is_err());
        let mut facts = native_facts();
        facts.0.get_mut(Path::new("/usr/bin")).unwrap().kind = PathKind::Symlink;
        assert!(verify_native_paths(&facts).is_err());
        let mut facts = native_facts();
        facts.0.get_mut(Path::new(RUNTIME)).unwrap().mount_point = true;
        assert!(verify_native_paths(&facts).is_err());
        let mut facts = native_facts();
        facts.0.remove(Path::new(RUNTIME));
        assert!(verify_native_paths(&facts).is_err());
    }

    #[test]
    fn native_package_state_requires_complete_unheld_amd64_installation() {
        assert_eq!(
            parse_installed_state("install ok installed\n2.0.0~beta.2-1\namd64").unwrap(),
            "2.0.0~beta.2-1"
        );
        for invalid in [
            "hold ok installed\n2.0.0-1\namd64",
            "install ok unpacked\n2.0.0-1\namd64",
            "install ok installed\n2.0.0-1\narm64",
            "install ok installed\n2.0.0-1\namd64\nextra",
        ] {
            assert!(parse_installed_state(invalid).is_err());
        }
    }

    #[test]
    fn apt_transactions_use_a_fixed_native_command_and_clean_environment() {
        let command = apt_command(
            Path::new("/run/lg-buddy-package-updates/staging/lg-buddy_2.0.0-1_amd64.deb"),
            false,
        );
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/apt-get"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--no-remove",
                "--no-install-recommends",
                "--no-allow-downgrades",
                "--no-allow-change-held-packages",
                "--no-allow-unauthenticated",
                "--yes",
                "install",
                "--",
                "/run/lg-buddy-package-updates/staging/lg-buddy_2.0.0-1_amd64.deb"
            ]
        );
        let environment = command.get_envs().collect::<Vec<_>>();
        assert_eq!(environment.len(), 4);
        assert!(environment.contains(&(
            OsStr::new("DEBIAN_FRONTEND"),
            Some(OsStr::new("noninteractive"))
        )));
        assert!(environment.iter().all(|(key, _)| !matches!(
            key.to_str(),
            Some("APT_CONFIG" | "DPKG_ROOT" | "LG_BUDDY_CONFIG")
        )));
        let simulation = apt_command(Path::new("/fixed/package.deb"), true);
        assert!(simulation.get_args().any(|arg| arg == "--simulate"));
        assert!(!simulation.get_args().any(|arg| arg == "--yes"));
    }

    #[test]
    fn private_entrypoint_refuses_bad_or_caller_selected_commands_before_io() {
        for args in [
            vec![],
            vec!["/tmp/package.deb".into()],
            vec!["v2.0.0".into(), "not-a-commit".into()],
            vec!["2.0.0".into(), "a".repeat(40)],
        ] {
            assert!(run(&args).is_err());
        }
    }
}
