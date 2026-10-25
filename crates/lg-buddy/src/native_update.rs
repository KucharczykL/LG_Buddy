//! Shared native update command and installed-path boundary.
use crate::upgrade_preflight::{FilesystemFacts, PathKind};
use std::path::Path;
use std::process::{Command, Stdio};
const RUNTIME: &str = "/usr/bin/lg-buddy";
const GUI: &str = "/usr/bin/lg-buddy-gui";
pub(super) fn native_command(program: &str) -> Command {
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

pub(super) fn capture(command: Command) -> Result<String, String> {
    let output =
        crate::update_install::run_bounded_command(command).map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(output.diagnostic());
    }
    Ok(output.stdout)
}

pub(super) fn verify_native_paths(filesystem: &impl FilesystemFacts) -> Result<(), String> {
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

pub(super) fn installer_command(
    package: &crate::release_bundle::native_package::VerifiedNativePackage,
    protocol: &str,
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
        .arg(protocol)
        .arg(package.identity.release_tag())
        .arg(package.identity.commit())
        .stdin(Stdio::from(
            package.input().map_err(|error| error.to_string())?,
        ));
    Ok(command)
}
