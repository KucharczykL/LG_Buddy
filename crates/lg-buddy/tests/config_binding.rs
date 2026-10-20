mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::TestConfigFile;

const PROFILE: &str = "tvs_primary_platform=lg_webos\ntvs_primary_ip=192.0.2.21\ntvs_primary_mac=02:00:00:00:00:21\ntvs_primary_input=HDMI_2\n";

struct Fixture {
    _guard: TestConfigFile,
    root: PathBuf,
    home: PathBuf,
    custom: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let guard = TestConfigFile::new("config-binding");
        let directory = guard.path().parent().unwrap();
        let root = directory.join("root");
        let home = directory.join("user-b");
        let custom = directory.join("custom-user-a/config.env");
        fs::create_dir_all(home.join(".config/lg-buddy")).unwrap();
        fs::create_dir_all(custom.parent().unwrap()).unwrap();
        fs::write(home.join(".config/lg-buddy/config.env"), PROFILE).unwrap();
        fs::write(&custom, PROFILE.replace("192.0.2.21", "192.0.2.10")).unwrap();
        Self {
            _guard: guard,
            root,
            home,
            custom,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lg-buddy"));
        command
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("LG_BUDDY_CONFIG")
            .env_remove("SUDO_USER")
            .env_remove("SUDO_UID");
        command
    }
    fn seed(&self, relative: &str, config: &Path) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("{}\n", config.display())).unwrap();
        path
    }
    fn sandbox(&self, binding: Option<&Path>, arguments: &[&str]) -> Output {
        // Root only inside the user namespace, so fixture binding ownership is
        // production-realistic without modifying the host /etc or /usr.
        let sandbox_root = self.root.with_file_name("sandbox-root");
        fs::create_dir_all(&sandbox_root).unwrap();
        let mut command = Command::new("bwrap");
        command
            .args(["--unshare-user", "--uid", "0", "--gid", "0", "--bind"])
            .arg(&sandbox_root)
            .arg("/");
        // The root inode must belong to namespace root too. Binding the host /
        // would expose its unmapped owner rather than model a root-owned tree.
        for dependency in ["/nix", "/lib", "/lib64", "/home", "/tmp", "/run"] {
            if Path::new(dependency).exists() {
                command.args(["--ro-bind", dependency, dependency]);
            }
        }
        command.args([
            "--dev",
            "/dev",
            "--tmpfs",
            "/etc",
            "--chmod",
            "0755",
            "/etc",
            "--dir",
            "/etc/lg-buddy",
            "--tmpfs",
            "/usr",
            "--chmod",
            "0755",
            "/usr",
            "--dir",
            "/usr/lib/lg-buddy",
        ]);
        let legacy = self.seed("usr/lib/lg-buddy/config-path", &self.custom);
        command
            .arg("--ro-bind")
            .arg(legacy)
            .arg("/usr/lib/lg-buddy/config-path");
        if let Some(binding) = binding {
            command
                .arg("--ro-bind")
                .arg(binding)
                .arg("/etc/lg-buddy/config-path");
        }
        command
            .arg(env!("CARGO_BIN_EXE_lg-buddy"))
            .args(arguments)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("LG_BUDDY_CONFIG")
            .env_remove("SUDO_USER")
            .env_remove("SUDO_UID");
        command.output().expect("run isolated config discovery")
    }
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn user_launch_ignores_both_machine_records_and_explicit_config_wins() {
    let fixture = Fixture::new();
    let binding = fixture.seed("etc/lg-buddy/config-path", &fixture.custom);
    assert_eq!(
        success(fixture.sandbox(Some(&binding), &["settings", "get", "tv.ip"])).trim(),
        "192.0.2.21"
    );
    assert_eq!(
        success(
            fixture
                .command()
                .env("LG_BUDDY_CONFIG", &fixture.custom)
                .args(["settings", "get", "tv.ip"])
                .output()
                .unwrap()
        )
        .trim(),
        "192.0.2.10"
    );
}

#[test]
fn remembered_custom_location_survives_later_launches_without_copying_settings() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.custom).unwrap();
    success(
        fixture
            .command()
            .args(["config-binding", "remember"])
            .arg(&fixture.custom)
            .output()
            .unwrap(),
    );
    assert_eq!(
        success(
            fixture
                .command()
                .args(["settings", "get", "tv.ip"])
                .output()
                .unwrap()
        )
        .trim(),
        "192.0.2.10"
    );
    assert_eq!(fs::read(&fixture.custom).unwrap(), before);
    assert!(!fixture.root.join("etc/lg-buddy/config-path").exists());
    assert_eq!(
        fs::read_to_string(fixture.home.join(".config/lg-buddy/config.env")).unwrap(),
        PROFILE
    );
    success(
        fixture
            .command()
            .args(["settings", "set", "updates.channel", "prerelease"])
            .output()
            .unwrap(),
    );
    assert!(!fixture.root.join("etc/lg-buddy/config-path").exists());
}

#[test]
fn nm_pre_down_uses_machine_binding_and_the_shared_stale_config_gate() {
    let fixture = Fixture::new();
    fs::write(&fixture.custom, PROFILE.replace("lg_webos", "bscpylgtv")).unwrap();
    let binding = fixture.seed("etc/lg-buddy/config-path", &fixture.custom);
    let output = fixture.sandbox(Some(&binding), &["nm-pre-down"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("v2 migration required"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nm_pre_down_missing_or_invalid_new_binding_never_uses_legacy_or_home() {
    let fixture = Fixture::new();
    let invalid = fixture.seed("etc/lg-buddy/config-path", &fixture.custom);
    fs::write(&invalid, "relative/config.env\n").unwrap();
    for binding in [None, Some(invalid.as_path())] {
        let output = fixture.sandbox(binding, &["nm-pre-down"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("config binding"));
    }
}

#[test]
fn installer_refuses_conflicting_explicit_selection_before_migration() {
    let fixture = Fixture::new();
    let pointer = fixture.seed("usr/lib/lg-buddy/config-path", &fixture.custom);
    let before = fs::read(&pointer).unwrap();
    let output = fixture
        .command()
        .env(
            "LG_BUDDY_CONFIG",
            fixture.home.join(".config/lg-buddy/config.env"),
        )
        .args(["config-binding", "inspect"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflicts"));
    assert_eq!(fs::read(pointer).unwrap(), before);
    assert!(!fixture.root.join("etc/lg-buddy/config-path").exists());
}

#[test]
fn binding_directories_are_safe_with_a_restrictive_installer_umask() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let config_home = fixture.home.join("new-config-home");
    let output = Command::new("sh")
        .args([
            "-c",
            "umask 077; exec \"$@\"",
            "config-binding-umask",
            env!("CARGO_BIN_EXE_lg-buddy"),
            "config-binding",
            "remember",
        ])
        .arg(&fixture.custom)
        .env("HOME", &fixture.home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env_remove("SUDO_USER")
        .env_remove("SUDO_UID")
        .output()
        .unwrap();
    success(output);
    for directory in [&config_home, &config_home.join("lg-buddy")] {
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
