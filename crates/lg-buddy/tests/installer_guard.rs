mod support;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::TestConfigFile;

const PROFILE: &str = "tvs_primary_platform=lg_webos\ntvs_primary_ip=192.0.2.21\ntvs_primary_mac=02:00:00:00:00:21\ntvs_primary_input=HDMI_2\nscreen_idle_blank=disabled\nsystem_sleep_wake_policy=disabled\nupdates_auto_check=disabled\n";

fn write(path: &Path, contents: &str, mode: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

struct Fixture {
    _guard: TestConfigFile,
    directory: PathBuf,
    root: PathBuf,
    home: PathBuf,
    bundle: PathBuf,
}
impl Fixture {
    fn new(configured: bool) -> Self {
        let guard = TestConfigFile::new("installer-guard");
        let directory = guard.path().parent().unwrap().to_owned();
        let root = directory.join("root with spaces");
        let home = directory.join("home");
        let bundle = directory.join("bundle");
        for path in [&root, &home, &directory.join("tmp")] {
            fs::create_dir_all(path).unwrap();
        }
        if configured {
            write(&home.join(".config/lg-buddy/config.env"), PROFILE, 0o600);
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            "install.sh",
            "bin/LG_Buddy_Common",
            "data/setup-services.sh",
            "data/io.github.staphylococcus.LGBuddy.setup.policy",
            "io.github.staphylococcus.LGBuddy.desktop",
        ] {
            let target = bundle.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(repo.join(relative), target).unwrap();
        }
        for entry in fs::read_dir(repo.join("systemd")).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                fs::create_dir_all(bundle.join("systemd")).unwrap();
                fs::copy(entry.path(), bundle.join("systemd").join(entry.file_name())).unwrap();
            }
        }
        write(
            &bundle.join("docs/io.github.staphylococcus.LGBuddy.svg"),
            "<svg/>",
            0o644,
        );
        write(
            &bundle.join("data/kwin/setup.sh"),
            "#!/bin/sh\nexit 0\n",
            0o755,
        );
        write(
            &bundle.join("data/kwin/build.sh"),
            "#!/bin/sh\nexit 0\n",
            0o755,
        );
        write(
            &bundle.join("data/kwin/prebuilt/plugin.so"),
            "candidate plugin",
            0o644,
        );
        write(
            &bundle.join("lg-buddy"),
            r#"#!/bin/sh
case "${1:-}" in
    installer-preflight|config-binding|settings|--version)
        exec "$TEST_REAL_RUNTIME" "$@" ;;
    upgrade-preflight) exit 0 ;;
    ""|setup) : > "$TEST_HANDOFF" ;;
    *) exit 2 ;;
esac
"#,
            0o755,
        );
        write(&bundle.join("docs/lg-buddy-gui-x86_64-unknown-linux-gnu"), "#!/bin/sh\n[ \"${1:-}\" = --version ] || exit 0\nexec \"$TEST_REAL_RUNTIME\" --version\n", 0o755);
        write(
            &directory.join("database"),
            r#"#!/bin/sh
for path do :; done
case "$0" in
    */rpm) printf 'file %s is not owned by any package\n' "$path" >&2; exit 1 ;;
    */pacman) printf 'error: No package owns %s\n' "$path" >&2; exit 1 ;;
esac
printf '%s\n' "$path" >> "$TEST_QUERY_LOG"
if [ "$path" = "${TEST_OWNED_PATH:-}" ]; then
    case "${TEST_DB_MODE:-owned}" in
        owned) printf 'fixture-owner: %s\n' "$path"; exit 0 ;;
        conflicting) printf 'fixture-one: %s\nfixture-two: %s\n' "$path" "$path"; exit 0 ;;
        error) echo 'fixture database unavailable' >&2; exit 2 ;;
    esac
fi
exit 1
"#,
            0o755,
        );
        for tool in ["pkexec", "systemctl", "systemd-tmpfiles"] {
            write(
                &directory.join("stubs").join(tool),
                "#!/bin/sh\n: > \"$TEST_SERVICE_ACTION\"\nexit 0\n",
                0o755,
            );
        }
        write(
            &directory.join("stubs/gui-probe"),
            "#!/bin/sh\n: > \"$TEST_PREREQUISITE_PROBE\"\nexit 0\n",
            0o755,
        );
        Self {
            _guard: guard,
            directory,
            root,
            home,
            bundle,
        }
    }

    fn sandbox(&self, read_only: bool) -> Command {
        let bwrap = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("bwrap"))
            .find(|path| path.is_file())
            .expect("bubblewrap is required for installer isolation");
        let mut command = Command::new(bwrap);
        command
            .args(["--ro-bind", "/", "/", "--bind"])
            .arg(&self.directory)
            .arg(&self.directory)
            .args(["--tmpfs", "/usr/bin"]);
        // Hide real package tools, supplying contract fixtures at the exact
        // absolute paths used by PackageDatabase::system. No production bypass.
        for tool in [
            "bash", "sh", "env", "id", "dirname", "readlink", "cat", "mkdir", "chmod", "cp",
            "install", "rm", "rmdir", "find", "mktemp", "sed", "grep", "stat", "cut", "getent",
            "tee", "touch", "uname", "cmp",
        ] {
            let source = std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|directory| directory.join(tool))
                .find(|path| path.is_file())
                .unwrap();
            command
                .arg("--ro-bind")
                .arg(source)
                .arg(format!("/usr/bin/{tool}"));
        }
        for tool in ["dpkg-query", "rpm", "pacman"] {
            command
                .arg("--ro-bind")
                .arg(self.directory.join("database"))
                .arg(format!("/usr/bin/{tool}"));
        }
        if read_only {
            command.arg("--ro-bind").arg(&self.root).arg(&self.root);
        }
        command
            .args(["--proc", "/proc", "--dev", "/dev"])
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.directory.join("stubs").display()),
            )
            .env("TEST_REAL_RUNTIME", env!("CARGO_BIN_EXE_lg-buddy"))
            .env("TEST_SERVICE_ACTION", self.directory.join("service-action"))
            .env("TEST_HANDOFF", self.directory.join("handoff"))
            .env("TEST_QUERY_LOG", self.directory.join("ownership-queries"))
            .env(
                "TEST_PREREQUISITE_PROBE",
                self.directory.join("prerequisite-probe"),
            )
            .env("TMPDIR", self.directory.join("tmp"))
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("LG_BUDDY_CONFIG")
            .env_remove("SUDO_USER")
            .env_remove("SUDO_UID")
            .env("LG_BUDDY_INSTALL_ROOT", &self.root)
            .env("LG_BUDDY_SUDO_CMD", "none")
            .env("LG_BUDDY_NONINTERACTIVE", "1")
            .env("LG_BUDDY_SKIP_SYSTEMD_ACTIONS", "1")
            .env(
                "LG_BUDDY_GUI_RUNTIME_PROBE",
                self.directory.join("stubs/gui-probe"),
            );
        // true is also needed by the installer GUI prerequisite probe.
        let source = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("true"))
            .find(|path| path.is_file())
            .unwrap();
        command.arg("--ro-bind").arg(source).arg("/usr/bin/true");
        command
    }

    fn install(&self, owned: &str, mode: &str, upgrade: bool, read_only: bool) -> Output {
        let mut command = self.sandbox(read_only);
        command
            .env("TEST_OWNED_PATH", self.root.join(owned))
            .env("TEST_DB_MODE", mode)
            .arg("/usr/bin/bash")
            .arg(self.bundle.join("install.sh"));
        if upgrade {
            command.arg("--upgrade");
        }
        command.output().unwrap()
    }
}

// Include bytes, link targets, owners and modes; reads may change access times.
fn snapshot(root: &Path) -> Vec<(PathBuf, u32, u32, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, result: &mut Vec<(PathBuf, u32, u32, Vec<u8>)>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let bytes = if metadata.file_type().is_symlink() {
            fs::read_link(path)
                .unwrap()
                .into_os_string()
                .into_encoded_bytes()
        } else if metadata.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        result.push((
            path.strip_prefix(root).unwrap().to_owned(),
            metadata.mode(),
            metadata.uid(),
            bytes,
        ));
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), result);
            }
        }
    }
    let mut result = Vec::new();
    visit(root, root, &mut result);
    result.sort();
    result
}

fn assert_installed_files_were_checked(fixture: &Fixture) {
    let queries = fs::read_to_string(fixture.directory.join("ownership-queries")).unwrap();
    for (relative, _, _, _) in snapshot(&fixture.root) {
        let path = fixture.root.join(relative);
        if fs::symlink_metadata(&path).unwrap().is_file() {
            assert!(
                queries.lines().any(|query| Path::new(query) == path),
                "installed without an ownership check: {}",
                path.display()
            );
        }
    }
}

#[test]
fn installer_refusals_leave_fresh_existing_and_upgrade_state_unchanged() {
    for (configured, upgrade) in [(false, false), (true, false), (true, true)] {
        for (owned, mode, read_only, expected) in [
            (
                "etc/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle",
                "owned",
                false,
                "fixture-owner",
            ),
            (
                "usr/share/icons/hicolor/scalable/apps/io.github.staphylococcus.LGBuddy.svg",
                "conflicting",
                false,
                "conflicting package ownership",
            ),
            ("usr/bin/lg-buddy-gui", "error", false, "probe failed"),
            (
                "usr/lib/lg-buddy/kwin/prebuilt/plugin.so",
                "owned",
                false,
                "fixture-owner",
            ),
            ("unowned", "owned", true, "mounted read-only"),
        ] {
            // Fresh installs defer machine service setup, so that destination
            // belongs to a later user-authorized flow, not this install plan.
            if !configured && owned.starts_with("etc/NetworkManager") {
                continue;
            }
            let f = Fixture::new(configured);
            if owned.starts_with("usr/share/") {
                write(&f.root.join(owned), "existing package asset", 0o644);
            }
            if upgrade {
                write(
                    &f.root.join("usr/lib/lg-buddy/config-path"),
                    &format!("{}\n", f.home.join(".config/lg-buddy/config.env").display()),
                    0o644,
                );
            }
            let root_before = snapshot(&f.root);
            let home_before = snapshot(&f.home);
            let output = f.install(owned, mode, upgrade, read_only);
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.status.success(), "{text}");
            assert!(text.contains(expected), "{text}");
            assert_eq!(snapshot(&f.root), root_before, "{text}");
            assert_eq!(snapshot(&f.home), home_before, "{text}");
            assert!(!f.directory.join("service-action").exists(), "{text}");
            assert!(!f.directory.join("handoff").exists(), "{text}");
            assert!(!f.directory.join("prerequisite-probe").exists(), "{text}");
        }
    }
}

#[test]
fn authorized_upgrade_helper_rechecks_the_entire_plan_before_mutation() {
    let f = Fixture::new(true);
    write(
        &f.root.join("usr/lib/lg-buddy/config-path"),
        &format!("{}\n", f.home.join(".config/lg-buddy/config.env").display()),
        0o644,
    );
    write(&f.directory.join("override.conf"), "[Service]\n", 0o600);
    write(&f.directory.join("nm-hook"), "#!/bin/sh\n", 0o700);
    let root_before = snapshot(&f.root);
    let home_before = snapshot(&f.home);
    let output = f
        .sandbox(false)
        .args(["--unshare-user", "--uid", "0", "--gid", "0"])
        .env(
            "TEST_OWNED_PATH",
            f.root
                .join("etc/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle"),
        )
        .arg("/usr/bin/bash")
        .arg(f.bundle.join("install.sh"))
        .arg("--system-upgrade")
        .arg(&f.root)
        .arg(&f.bundle)
        .arg(f.directory.join("override.conf"))
        .arg(f.directory.join("nm-hook"))
        .args(["1", "1"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("LG_BUDDY_INSTALL_STATUS=authorized"),
        "{text}"
    );
    assert!(text.contains("fixture-owner"), "{text}");
    assert!(
        !text.contains("LG_BUDDY_INSTALL_STATUS=mutation_started"),
        "{text}"
    );
    assert_eq!(snapshot(&f.root), root_before);
    assert_eq!(snapshot(&f.home), home_before);
}

#[test]
fn unowned_bundle_install_and_upgrade_still_work() {
    for configured in [false, true] {
        let f = Fixture::new(configured);
        let shared = f.root.join("usr/bin");
        fs::create_dir_all(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o711)).unwrap();
        let output = f.install("usr/bin", "owned", false, false);
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(f.root.join("usr/bin/lg-buddy").is_file());
        assert!(f.root.join("etc/lg-buddy/config-path").is_file());
        assert_installed_files_were_checked(&f);
        assert_eq!(fs::metadata(&shared).unwrap().mode() & 0o777, 0o711);
        if !configured {
            assert!(f.directory.join("handoff").is_file());
        }
        if configured {
            let config = fs::read(f.home.join(".config/lg-buddy/config.env")).unwrap();
            fs::remove_file(f.directory.join("ownership-queries")).unwrap();
            let output = f.install("usr/bin", "owned", true, false);
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read(f.home.join(".config/lg-buddy/config.env")).unwrap(),
                config
            );
            assert_installed_files_were_checked(&f);
            assert_eq!(fs::metadata(&shared).unwrap().mode() & 0o777, 0o711);
        }
    }
}
