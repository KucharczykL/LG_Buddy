mod support;

use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::TestConfigFile;

const PLUGIN_ROOT: &str = "/usr/lib/qt6/plugins";
const PAYLOAD: &[u8] = b"verified KWin artifact";

struct Fixture {
    _guard: TestConfigFile,
    directory: PathBuf,
    plugins: PathBuf,
    destination: PathBuf,
    id: String,
}

impl Fixture {
    fn new(existing: bool) -> Self {
        let guard = TestConfigFile::new("kwin-ownership");
        let directory = guard.path().parent().unwrap().to_owned();
        let plugins = directory.join("plugins");
        fs::create_dir_all(plugins.join("kwin/plugins")).unwrap();
        fs::create_dir_all(directory.join("etc")).unwrap();
        fs::create_dir_all(directory.join("run")).unwrap();
        fs::write(directory.join("plugin.so"), PAYLOAD).unwrap();
        let hash = format!("{:x}", Sha256::digest(PAYLOAD));
        let id = format!("lg_buddy_inhibition_424242_{hash}");
        let destination = plugins.join("kwin/plugins").join(format!("{id}.so"));
        if existing {
            fs::write(&destination, PAYLOAD).unwrap();
        }
        let database = directory.join("database");
        fs::write(
            &database,
            r#"#!/usr/bin/sh
for path do :; done
printf '%s\n' "$path" >> "$TEST_QUERY_LOG"
tool=${0##*/}
case "$TEST_DB_MODE:$tool" in
    error:dpkg-query) echo 'fixture database unavailable' >&2; exit 2 ;;
    conflicting:dpkg-query) printf 'one: %s\ntwo: %s\n' "$path" "$path"; exit 0 ;;
    dpkg-owned:dpkg-query) printf 'kwin-package: %s\n' "$path"; exit 0 ;;
    rpm-owned:rpm|pacman-owned:pacman) printf 'kwin-package\n'; exit 0 ;;
    directory-owned:dpkg-query)
        if [ "$path" = /usr/lib/qt6/plugins/kwin/plugins ]; then
            printf 'kwin-package: %s\n' "$path"; exit 0
        fi ;;
esac
case "$tool" in
    rpm) printf 'file %s is not owned by any package\n' "$path" ;;
    pacman) printf 'error: No package owns %s\n' "$path" >&2 ;;
esac
exit 1
"#,
        )
        .unwrap();
        fs::set_permissions(database, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _guard: guard,
            directory,
            plugins,
            destination,
            id,
        }
    }

    fn action(&self, remove: bool, mode: &str, read_only: bool) -> Output {
        let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("sh"))
            .find(|path| path.is_file())
            .unwrap();
        // Namespace root exercises the actual privileged entrypoint. Bind only
        // fixture directories over its fixed host paths; the host stays intact.
        let mut command = Command::new("bwrap");
        command.args(["--unshare-user", "--uid", "0", "--gid", "0", "--tmpfs", "/"]);
        // Keep the host toolchain/loader available on both Nix and FHS hosts,
        // without inheriting their /usr, /etc or immutable-host markers.
        for dependency in ["/nix", "/lib", "/lib64"] {
            if Path::new(dependency).exists() {
                command.args(["--ro-bind", dependency, dependency]);
            }
        }
        command
            .arg("--bind")
            .arg(&self.directory)
            .arg(&self.directory)
            .args(["--dir", "/usr/bin", "--ro-bind"])
            .arg(fs::canonicalize(shell).unwrap())
            .arg("/usr/bin/sh")
            .args(["--dir", "/usr/sbin"])
            .arg(if read_only { "--ro-bind" } else { "--bind" })
            .arg(&self.plugins)
            .arg(PLUGIN_ROOT)
            .arg("--bind")
            .arg(self.directory.join("etc"))
            .arg("/etc")
            .arg("--bind")
            .arg(self.directory.join("run"))
            .arg("/run")
            .args(["--proc", "/proc", "--dev", "/dev", "--ro-bind"])
            .arg(env!("CARGO_BIN_EXE_lg-buddy"))
            .arg("/runtime/bin/lg-buddy");
        if mode != "unavailable" {
            for tool in ["dpkg-query", "rpm", "pacman"] {
                command
                    .arg("--ro-bind")
                    .arg(self.directory.join("database"))
                    .arg(format!("/usr/bin/{tool}"));
            }
        }
        command
            .env("TEST_DB_MODE", mode)
            .env("TEST_QUERY_LOG", self.directory.join("queries"))
            .arg("/runtime/bin/lg-buddy")
            .args([
                "kwin-setup",
                if remove {
                    "--system-remove"
                } else {
                    "--system-install"
                },
                "424242",
                PLUGIN_ROOT,
            ])
            .arg(&self.id);
        if !remove {
            command.arg(self.directory.join("plugin.so"));
        }
        command.output().unwrap()
    }
}

#[test]
fn mutable_unowned_kwin_artifacts_can_be_installed_verified_and_removed() {
    let f = Fixture::new(false);
    let directory = f.destination.parent().unwrap();
    let before = fs::metadata(directory).unwrap();
    for remove in [false, false, true] {
        let output = f.action(remove, "directory-owned", false);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !remove {
            assert_eq!(fs::read(&f.destination).unwrap(), PAYLOAD);
            assert_eq!(
                fs::metadata(&f.destination).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }
        let after = fs::metadata(directory).unwrap();
        assert_eq!((after.uid(), after.mode()), (before.uid(), before.mode()));
    }
    assert!(!f.destination.exists());
    let queries = fs::read_to_string(f.directory.join("queries")).unwrap();
    assert_eq!(queries.lines().count(), 9);
    assert!(queries.lines().all(|path| Path::new(path)
        == Path::new(PLUGIN_ROOT)
            .join("kwin/plugins")
            .join(format!("{}.so", f.id))));
}

#[test]
fn privileged_kwin_actions_refuse_owned_conflicting_unverifiable_and_read_only_targets() {
    for remove in [false, true] {
        for (mode, read_only, expected) in [
            ("dpkg-owned", false, "owned by dpkg package"),
            ("rpm-owned", false, "owned by rpm package"),
            ("pacman-owned", false, "owned by pacman package"),
            ("conflicting", false, "conflicting package ownership"),
            ("error", false, "probe failed"),
            ("unavailable", false, "no package database"),
            ("unowned", true, "read-only"),
        ] {
            let f = Fixture::new(remove);
            let before = fs::read(&f.destination).ok();
            let output = f.action(remove, mode, read_only);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{mode}/{remove}: {stderr}");
            assert!(stderr.contains(expected), "{mode}/{remove}: {stderr}");
            assert_eq!(fs::read(&f.destination).ok(), before);
        }
    }
}

#[test]
fn immutable_kwin_hosts_refuse_privileged_mutation() {
    for marker in ["etc/NIXOS", "run/ostree-booted"] {
        for remove in [false, true] {
            let f = Fixture::new(remove);
            fs::write(f.directory.join(marker), "managed host").unwrap();
            let before = fs::read(&f.destination).ok();
            let output = f.action(remove, "unowned", false);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{stderr}");
            assert!(stderr.contains("immutable/declarative"), "{stderr}");
            assert_eq!(fs::read(&f.destination).ok(), before);
            assert!(!f.directory.join("queries").exists());
        }
    }
}

#[test]
fn redirected_kwin_directories_cannot_mutate_files_outside_the_loader_target() {
    for remove in [false, true] {
        let f = Fixture::new(false);
        let external = f.directory.join("external");
        fs::create_dir_all(external.join("plugins")).unwrap();
        let artifact = external.join("plugins").join(format!("{}.so", f.id));
        if remove {
            fs::write(&artifact, PAYLOAD).unwrap();
        }
        fs::remove_dir_all(f.plugins.join("kwin")).unwrap();
        std::os::unix::fs::symlink(&external, f.plugins.join("kwin")).unwrap();
        let before = fs::read(&artifact).ok();
        let output = f.action(remove, "unowned", false);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{stderr}");
        assert!(stderr.contains("symlink"), "{stderr}");
        assert_eq!(fs::read(&artifact).ok(), before);
    }
}
