//! Native setup acceptance in disposable user/mount namespaces. The broker runs
//! the real privileged entrypoint in a sibling root namespace, never a fake copy.
//! KWin is an external contract fixture here; real loader acceptance runs in CI.
use crate::support::{dbus_daemon_path, TestConfigFile};
use dbus::{blocking::SyncConnection, channel::MatchingReceiver, message::MatchRule};
use dbus_crossroads::{Crossroads, MethodErr};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const SERVICE: &str = "io.github.staphylococcus.LGBuddy.KWinInhibition";
const PLUGIN: &[u8] = b"verified KWin acceptance artifact";

pub struct PlasmaSetup {
    _directory: TestConfigFile,
    root: PathBuf,
    plugins: PathBuf,
    source: String,
    address: String,
    daemon_pid: i32,
    stop: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
    succeeded: bool,
}

impl PlasmaSetup {
    pub fn new(layout: &str) -> Self {
        let directory = TestConfigFile::new("plasma-setup");
        let root = directory.path().parent().unwrap().to_owned();
        let lib = if layout == "real lib64" {
            "lib64"
        } else {
            "lib"
        };
        let plugins = root.join(format!("usr/{lib}/qt6/plugins/kwin/plugins"));
        for path in [
            &plugins,
            &root.join("usr/bin"),
            &root.join("home"),
            &root.join("payload/source"),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        if layout != "real lib64" {
            symlink("lib", root.join("usr/lib64")).unwrap();
        }
        let mut manifest = String::new();
        for name in ["CMakeLists.txt", "main.cpp", "metadata.json"] {
            let bytes = fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../data/kwin/source")
                    .join(name),
            )
            .unwrap();
            manifest.push_str(&format!("{:x}  {name}\n", Sha256::digest(&bytes)));
            fs::write(root.join("payload/source").join(name), bytes).unwrap();
        }
        let source = format!("{:x}", Sha256::digest(manifest));
        fs::create_dir_all(root.join("payload/prebuilt/current")).unwrap();
        fs::write(root.join("payload/prebuilt/current/plugin.so"), PLUGIN).unwrap();
        fs::write(
            root.join("payload/prebuilt/current/metadata.tsv"),
            format!(
                "6.7.5\t6.11.2\t{}\t{source}\t{:x}\n",
                std::env::consts::ARCH,
                Sha256::digest(PLUGIN)
            ),
        )
        .unwrap();
        // Only external package/configuration tools are substituted. The
        // installer itself performs its real safety checks and file operations.
        for (name, body) in [
            ("pacman", "for path do :; done\nprintf 'error: No package owns %s\\n' \"$path\" >&2\nexit 1\n"),
            ("kwriteconfig6", "exit 0\n"),
        ] {
            let path = root.join("usr/bin").join(name);
            fs::write(&path, format!("#!/usr/bin/sh\n{body}")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        if layout == "redirected Qt" {
            fs::rename(root.join("usr/lib/qt6"), root.join("external")).unwrap();
            symlink("/scenario/external", root.join("usr/lib/qt6")).unwrap();
            fs::write(
                root.join("external/plugins/kwin/plugins/vendor.so"),
                b"vendor plugin",
            )
            .unwrap();
        }
        // Namespace uid 1000 can differ from the runner uid. Authenticate this
        // private fixture bus anonymously so EXTERNAL's uid translation does
        // not hide the product behavior behind a fixture authentication error.
        let bus_config = root.join("bus.conf");
        fs::write(
            &bus_config,
            r#"<busconfig>
          <type>session</type><listen>unix:tmpdir=/tmp</listen>
          <auth>ANONYMOUS</auth><allow_anonymous/>
          <policy context="default"><allow own="*"/>
            <allow send_destination="*"/><allow receive_sender="*"/>
          </policy>
        </busconfig>"#,
        )
        .unwrap();
        let output = Command::new(dbus_daemon_path())
            .arg("--config-file")
            .arg(bus_config)
            .args([
                "--fork",
                "--print-address=1",
                "--print-pid=1",
                "--nopidfile",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        let mut fields = output.lines();
        let address = fields.next().unwrap().to_owned();
        let daemon_pid = fields.next().unwrap().parse().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server = Some(kwin(&address, &plugins, &source, Arc::clone(&stop)));
        Self {
            _directory: directory,
            root,
            plugins,
            source,
            address,
            daemon_pid,
            stop,
            server,
            succeeded: false,
        }
    }

    fn sandbox(&self, uid: &str) -> Command {
        let mut command = Command::new("bwrap");
        command.args([
            "--unshare-user",
            "--uid",
            uid,
            "--gid",
            uid,
            "--die-with-parent",
            "--tmpfs",
            "/",
        ]);
        for path in ["/nix", "/lib", "/lib64"] {
            if Path::new(path).exists() {
                command.args(["--ro-bind", path, path]);
            }
        }
        let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        command
            .arg("--bind")
            .arg(&self.root)
            .arg("/scenario")
            .arg("--bind")
            .arg(self.root.join("usr"))
            .arg("/usr")
            .arg("--ro-bind")
            .arg(fs::canonicalize(shell).unwrap())
            .arg("/usr/bin/sh")
            .args([
                "--ro-bind",
                "/tmp",
                "/tmp",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
            ])
            .arg("--ro-bind")
            .arg(env!("CARGO_BIN_EXE_lg-buddy"))
            .arg("/runtime/bin/lg-buddy")
            .env("PATH", "/usr/bin")
            .env("HOME", "/scenario/home")
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address);
        command
    }

    pub fn previous_installation(&self) {
        let id = format!(
            "lg_buddy_inhibition_1000_{:x}",
            Sha256::digest(b"previous plugin")
        );
        fs::write(self.plugins.join(format!("{id}.so")), b"previous plugin").unwrap();
        let receipts = self.root.join("home/.local/state/lg-buddy/kwin/plugins");
        fs::create_dir_all(&receipts).unwrap();
        fs::write(
            receipts.join(format!("{id}.tsv")),
            format!("/usr/lib64/qt6/plugins\t{id}\n"),
        )
        .unwrap();
    }

    pub fn setup(&mut self, mode: &str) {
        let mut command = self.sandbox("1000");
        command.args(["/usr/bin/sh", "-c", "exec 5>/scenario/stdout 6>/scenario/stderr; exec /runtime/bin/lg-buddy kwin-setup --broker --payload-dir /scenario/payload \"$1\"", "worker", mode])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut requests = BufReader::new(child.stdout.take().unwrap());
        let mut replies = child.stdin.take().unwrap();
        while let Some(operation) = field(&mut requests) {
            let count: usize = field(&mut requests).unwrap().parse().unwrap();
            assert!(count <= 16);
            let args: Vec<_> = (0..count).map(|_| field(&mut requests).unwrap()).collect();
            let code = match operation.as_str() {
                "mutation" => {
                    assert!(args.is_empty());
                    0
                }
                "privileged" => {
                    let output = self
                        .sandbox("0")
                        .arg("/runtime/bin/lg-buddy")
                        .arg("kwin-setup")
                        .args(&args)
                        .output()
                        .unwrap();
                    fs::OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(self.root.join("privileged.log"))
                        .unwrap()
                        .write_all(&output.stderr)
                        .unwrap();
                    output.status.code().unwrap_or(1)
                }
                _ => panic!("unexpected broker operation: {operation}"),
            };
            write!(replies, "{code}\0").unwrap();
            replies.flush().unwrap();
        }
        drop(replies);
        let output = child.wait_with_output().unwrap();
        self.succeeded = output.status.success();
        assert!(
            output.stderr.is_empty(),
            "namespace failure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub fn assert_available(&self) {
        assert!(
            self.succeeded,
            "native setup failed:\n{}\n{}",
            self.log("stderr"),
            self.log("privileged.log")
        );
        let output = self
            .sandbox("1000")
            .args(["/runtime/bin/lg-buddy", "kwin-bridge", "check"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("6.7.5\t{}", self.source)
        );
        let installed: Vec<_> = fs::read_dir(&self.plugins).unwrap().collect();
        assert_eq!(
            installed.len(),
            1,
            "old plugins must be removed during repair"
        );
        assert_eq!(
            fs::read(installed[0].as_ref().unwrap().path()).unwrap(),
            PLUGIN
        );
        assert_eq!(
            fs::read_dir(self.root.join("home/.local/state/lg-buddy/kwin/plugins"))
                .unwrap()
                .count(),
            1
        );
    }

    pub fn assert_removed(&self) {
        assert!(self.succeeded, "{}", self.log("privileged.log"));
        assert_eq!(fs::read_dir(&self.plugins).unwrap().count(), 0);
        assert_eq!(
            fs::read_dir(self.root.join("home/.local/state/lg-buddy/kwin/plugins"))
                .unwrap()
                .count(),
            0
        );
        assert!(!self
            .sandbox("1000")
            .args(["/runtime/bin/lg-buddy", "kwin-bridge", "check"])
            .output()
            .unwrap()
            .status
            .success());
    }

    pub fn assert_refused(&self) {
        assert!(!self.succeeded);
        assert!(self.log("privileged.log").contains("is a symlink"));
        let external = self.root.join("external/plugins/kwin/plugins");
        assert_eq!(fs::read_dir(&external).unwrap().count(), 1);
        assert_eq!(
            fs::read(external.join("vendor.so")).unwrap(),
            b"vendor plugin"
        );
    }

    fn log(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap_or_default()
    }
}

fn field(input: &mut impl BufRead) -> Option<String> {
    let mut bytes = Vec::new();
    if input.read_until(0, &mut bytes).unwrap() == 0 {
        return None;
    }
    assert_eq!(bytes.pop(), Some(0));
    Some(String::from_utf8(bytes).unwrap())
}

fn kwin(address: &str, plugins: &Path, source: &str, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    let address = address.to_owned();
    let plugins = plugins.to_owned();
    let source = source.to_owned();
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let connection = Arc::new(SyncConnection::new_address(&address).unwrap());
        let loaded = Arc::new(Mutex::new(None::<String>));
        let mut crossroads = Crossroads::new();
        let info = crossroads.register("org.kde.KWin", |builder| {
            builder.method("supportInformation", (), ("info",), |_, _: &mut (), ()| {
                Ok((
                    "Operation Mode: Wayland\nKWin version: 6.7.5\nQt Version: 6.11.2\n".to_owned(),
                ))
            });
        });
        let loader = crossroads.register("org.kde.KWin.Plugins", |builder| {
            let load_connection = Arc::downgrade(&connection);
            let current = Arc::clone(&loaded);
            builder.method(
                "LoadPlugin",
                ("id",),
                ("loaded",),
                move |_, _: &mut (), (id,): (String,)| {
                    if fs::read(plugins.join(format!("{id}.so"))).ok().as_deref() != Some(PLUGIN) {
                        return Err(MethodErr::failed(
                            "plugin was not installed by native setup",
                        ));
                    }
                    load_connection
                        .upgrade()
                        .unwrap()
                        .request_name(SERVICE, false, true, false)
                        .unwrap();
                    *current.lock().unwrap() = Some(id);
                    Ok((true,))
                },
            );
            let unload_connection = Arc::downgrade(&connection);
            let current = Arc::clone(&loaded);
            builder.method(
                "UnloadPlugin",
                ("id",),
                (),
                move |_, _: &mut (), (id,): (String,)| {
                    if current.lock().unwrap().as_ref() == Some(&id) {
                        unload_connection
                            .upgrade()
                            .unwrap()
                            .release_name(SERVICE)
                            .unwrap();
                        *current.lock().unwrap() = None;
                    }
                    Ok(())
                },
            );
        });
        let bridge = crossroads.register(
            "io.github.staphylococcus.LGBuddy.KWinInhibition1",
            |builder| {
                builder.method("IsInhibited", (), ("inhibited",), |_, _: &mut (), ()| {
                    Ok((false,))
                });
                builder.method("BuildVersion", (), ("version",), |_, _: &mut (), ()| {
                    Ok(("6.7.5".to_owned(),))
                });
                builder.method("BuildId", (), ("id",), move |_, _: &mut (), ()| {
                    Ok((source.clone(),))
                });
            },
        );
        crossroads.insert("/KWin", &[info], ());
        crossroads.insert("/Plugins", &[loader], ());
        crossroads.insert(
            "/io/github/staphylococcus/LGBuddy/KWinInhibition",
            &[bridge],
            (),
        );
        let crossroads = Mutex::new(crossroads);
        connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |message, conn| {
                crossroads
                    .lock()
                    .unwrap()
                    .handle_message(message, conn)
                    .unwrap();
                true
            }),
        );
        connection
            .request_name("org.kde.KWin", false, true, false)
            .unwrap();
        ready_tx.send(()).unwrap();
        while !stop.load(Ordering::SeqCst) {
            connection.process(Duration::from_millis(10)).unwrap();
        }
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    worker
}

impl Drop for PlasmaSetup {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.server.take().unwrap().join().unwrap();
        unsafe {
            libc::kill(self.daemon_pid, libc::SIGTERM);
        }
    }
}
