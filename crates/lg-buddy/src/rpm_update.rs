//! DNF owns the native transaction; only independently verified release bytes
//! cross the administrator boundary. A stored plan is checked before replay.
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::process::Command;

use crate::native_update::{capture, native_command, verify_native_paths};
use crate::release_bundle::{
    rpm, verify_release_binary_identity, verify_release_gui_binary_identity, ReleaseIdentity,
};
use crate::updates::UpdateChannel;
use crate::upgrade_preflight::{OsFilesystemFacts, PackageDatabase, PackageFamily, PathOwnership};
use semver::Version;
use serde::Deserialize;

pub(crate) fn installed_version() -> Result<String, String> {
    verify_native_paths(&OsFilesystemFacts)?;
    for path in ["/usr/bin/lg-buddy", "/usr/bin/lg-buddy-gui"] {
        match PackageDatabase::system().owner_of(Path::new(path)) {
            Ok(PathOwnership::Owned(owner))
                if owner.family == PackageFamily::Rpm && owner.name == "lg-buddy" => {}
            _ => {
                return Err(format!(
                    "DNF update requires both binaries to belong to the lg-buddy RPM: {path}"
                ))
            }
        }
    }
    for path in [
        "/usr/bin/dnf5",
        "/usr/bin/rpm",
        "/usr/bin/rpmkeys",
        "/usr/bin/rpm2archive",
    ] {
        if !Path::new(path).is_file() {
            return Err(format!("Native update tool unavailable: {path}"));
        }
    }
    let mut command = native_command("/usr/bin/rpm");
    command.args([
        "--query",
        "--queryformat",
        "%{NAME}\n%{EPOCHNUM}\n%{VERSION}-%{RELEASE}\n%{ARCH}\n",
        "lg-buddy",
    ]);
    parse_installed_state(&capture(command)?)
}

fn parse_installed_state(text: &str) -> Result<String, String> {
    match text.lines().collect::<Vec<_>>().as_slice() {
        ["lg-buddy", "0", version, "x86_64"] if parse_native_version(version).is_ok() => Ok(version.to_string()),
        _ => Err("Expected one fully installed lg-buddy RPM with epoch 0, canonical version and architecture x86_64; inspect its RPM/DNF state".into()),
    }
}

fn parse_native_version(text: &str) -> Result<(Version, u64), String> {
    let (version, revision) = text.rsplit_once('-').ok_or("Malformed RPM version")?;
    let version =
        Version::parse(&version.replacen('~', "-", 1)).map_err(|_| "Malformed RPM version")?;
    let release = revision
        .parse::<u64>()
        .map_err(|_| "Malformed RPM revision")?;
    if release == 0
        || release.to_string() != revision
        || rpm::native_version(&version).map_err(|e| e.to_string())?
            != format!("{}-1", text.rsplit_once('-').unwrap().0)
    {
        return Err("Unsupported RPM version or revision".into());
    }
    Ok((version, release))
}

pub(crate) fn candidate_preflight(expected: &ReleaseIdentity) -> Result<(), String> {
    let current = installed_version()?;
    let target = rpm::native_version(expected.version()).map_err(|e| e.to_string())?;
    if parse_native_version(&current)? >= parse_native_version(&target)? {
        return Err(format!(
            "Refusing a DNF update from {current} to {target}; start a fresh update check"
        ));
    }
    Ok(())
}

pub(crate) fn verify_installed(expected: &ReleaseIdentity) -> Result<(), String> {
    let observed = installed_version()?;
    let target = rpm::native_version(expected.version()).map_err(|e| e.to_string())?;
    if observed != target {
        return Err(format!("DNF installed {observed}, expected {target}"));
    }
    verify_release_binary_identity(Path::new("/usr/bin/lg-buddy"), expected)
        .map_err(|e| e.to_string())?;
    verify_release_gui_binary_identity(Path::new("/usr/bin/lg-buddy-gui"), expected)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn installer_command(
    package: &rpm::VerifiedRpmPackage,
    graphical: bool,
) -> Result<Command, String> {
    crate::native_update::installer_command(package, "apply-rpm-update", graphical)
}

fn dnf_command() -> Command {
    let mut command = native_command("/usr/bin/dnf5");
    command.args([
        "--assumeyes",
        "--setopt=install_weak_deps=False",
        "--setopt=obsoletes=False",
        "--setopt=tsflags=",
        "--setopt=*.gpgcheck=True",
    ]);
    command
}
fn store_command(package: &Path, transaction: &Path) -> Command {
    let mut command = dnf_command();
    command
        .args(["upgrade", "--no-allow-downgrade"])
        .arg(format!("--store={}", transaction.display()))
        .arg(package);
    command
}
fn replay_command(transaction: &Path) -> Command {
    let mut command = dnf_command();
    command.arg("replay").arg(transaction);
    command
}

fn signature_command(package: &Path) -> Command {
    let mut command = native_command("/usr/bin/rpmkeys");
    // Stored repositories are COMMANDLINE repositories in newer DNF, where
    // repository gpgcheck does not apply. Require both signatures and digests
    // against the system keyring before replay, even if RPM defaults allow
    // unsigned packages. Only our independently verified LG Buddy RPM is exempt.
    command
        .args([
            "--checksig",
            "--define",
            "_pkgverify_level all",
            "--define",
            "_pkgverify_flags 0x0",
        ])
        .arg(package);
    command
}

struct VerifiedPlan {
    local_package: String,
    repository_packages: Vec<String>,
}

fn source_repository(repo_id: &str) -> Result<&str, String> {
    // DNF >= 5.3 wraps stored repository IDs to avoid collisions with live
    // repositories. Accept exactly one wrapper, including for @commandline.
    let source = if let Some(inner) = repo_id.strip_prefix("@stored_transaction(") {
        inner
            .strip_suffix(')')
            .ok_or("Malformed stored DNF repository")?
    } else {
        repo_id
    };
    if source.is_empty()
        || source.contains('(')
        || source.contains(')')
        || source.starts_with("@stored_transaction")
    {
        return Err("Malformed stored DNF repository".into());
    }
    Ok(source)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: String,
    rpms: Vec<PlanPackage>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanPackage {
    nevra: String,
    action: String,
    reason: String,
    repo_id: String,
    package_path: Option<String>,
}
fn name_arch(nevra: &str) -> Option<(&str, &str)> {
    let (nvr, arch) = nevra.rsplit_once('.')?;
    let (nv, _) = nvr.rsplit_once('-')?;
    let (name, _) = nv.rsplit_once('-')?;
    if name.is_empty() || arch.is_empty() {
        return None;
    }
    Some((name, arch))
}
fn verify_plan(
    bytes: &[u8],
    current: &str,
    target: &str,
    asset_name: &str,
) -> Result<VerifiedPlan, String> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("DNF plan is too large".into());
    }
    let plan: Plan =
        serde_json::from_slice(bytes).map_err(|e| format!("Unsupported DNF plan: {e}"))?;
    if plan.version != "1.0" || plan.rpms.len() > 4096 {
        return Err("Unsupported DNF transaction format".into());
    }
    let wanted = format!("lg-buddy-{target}.x86_64");
    let old = format!("lg-buddy-{current}.x86_64");
    let local = format!("./packages/{asset_name}");
    let mut upgraded = std::collections::HashSet::new();
    let mut replaced = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut found = false;
    let mut repository_packages = Vec::new();
    for package in &plan.rpms {
        let key = name_arch(&package.nevra).ok_or("Malformed DNF package identity")?;
        if !seen.insert(package.nevra.as_str())
            || package.reason.is_empty()
            || package.repo_id.is_empty()
        {
            return Err("Ambiguous DNF package entry".into());
        }
        match package.action.as_str() {
            "Install" | "Upgrade" => {
                let repository = source_repository(&package.repo_id)?;
                let path = package
                    .package_path
                    .as_deref()
                    .ok_or("Missing DNF package path")?;
                let path = path
                    .strip_prefix("./packages/")
                    .ok_or("Unexpected DNF package location")?;
                if path.is_empty() || path.contains('/') || path == "." || path == ".." {
                    return Err("Unsafe DNF package location".into());
                }
                if package.action == "Upgrade" {
                    upgraded.insert(key);
                }
                if key.0 == "lg-buddy" {
                    if package.nevra != wanted
                        || package.action != "Upgrade"
                        || repository != "@commandline"
                        || package.package_path.as_deref() != Some(local.as_str())
                        || found
                    {
                        return Err("DNF did not select the verified LG Buddy upgrade".into());
                    }
                    found = true;
                } else {
                    if repository == "@commandline" || repository == "@System" {
                        return Err("Unexpected non-repository dependency in DNF plan".into());
                    }
                    repository_packages.push(package.package_path.clone().unwrap());
                }
            }
            "Replaced" => {
                if package.package_path.is_some()
                    || package.repo_id != "@System"
                    || !replaced.insert(key)
                    || (key.0 == "lg-buddy" && package.nevra != old)
                {
                    return Err("Unexpected DNF replacement".into());
                }
            }
            _ => {
                return Err(format!(
                    "Refusing DNF transaction action: {}",
                    package.action
                ))
            }
        }
    }
    if !found || !replaced.contains(&("lg-buddy", "x86_64")) || upgraded != replaced {
        return Err("DNF transaction changes package ownership or omitted the upgrade (check version locks)".into());
    }
    Ok(VerifiedPlan {
        local_package: local,
        repository_packages,
    })
}

fn verify_repository_packages(transaction: &Path, plan: &VerifiedPlan) -> Result<(), String> {
    for relative in &plan.repository_packages {
        let path = transaction.join(relative);
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err("Expected a regular stored repository RPM".into());
        }
        capture(signature_command(&path)).map_err(|e| {
            format!("Repository RPM signature/digest verification failed before DNF replay; check the system's trusted RPM keys: {e}")
        })?;
    }
    Ok(())
}

pub fn run(arguments: &[String]) -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("RPM installation requires administrator authorization".into());
    }
    let [tag, commit] = arguments else {
        return Err("Expected a release tag and full commit identity".into());
    };
    let version = tag
        .strip_prefix('v')
        .and_then(|s| Version::parse(s).ok())
        .ok_or("Invalid RPM release tag")?;
    if tag != &format!("v{version}")
        || commit.len() != 40
        || !commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid RPM release identity".into());
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
        .map_err(|e| e.to_string())?
        .into();
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("Expected a bounded regular package file on standard input".into());
    }
    let package = rpm::receive(&expected, input, Path::new("/run/lg-buddy-package-updates"))
        .map_err(|e| e.to_string())?;
    let transaction = package.path.parent().unwrap().join("transaction");
    capture(store_command(&package.path, &transaction))?;
    let mut plan = Vec::new();
    File::open(transaction.join("transaction.json"))
        .and_then(|f| f.take(2 * 1024 * 1024 + 1).read_to_end(&mut plan))
        .map_err(|e| format!("DNF did not produce a usable upgrade plan (check version locks and package state): {e}"))?;
    let current = installed_version()?;
    let target = rpm::native_version(expected.version()).map_err(|e| e.to_string())?;
    let asset_name = package
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid verified RPM filename")?;
    let plan = verify_plan(&plan, &current, &target, asset_name)?;
    verify_repository_packages(&transaction, &plan)?;
    // DNF --store does not copy @commandline RPMs. Populate only the checked
    // name from the independently verified, root-private receipt.
    let destination = transaction.join(&plan.local_package);
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::copy(&package.path, &destination).map_err(|e| e.to_string())?;
    candidate_preflight(&expected)?;
    println!("LG_BUDDY_INSTALL_STATUS=mutation_started");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let status = replay_command(&transaction)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("DNF transaction failed with status {status}; inspect and repair the package state before retrying"));
    }
    verify_installed(&expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn plan() -> serde_json::Value {
        json!({"version":"1.0","rpms":[
            {"nevra":"lg-buddy-2.0.0-1.x86_64","action":"Upgrade","reason":"User","repo_id":"@commandline","package_path":"./packages/lg-buddy-2.0.0-1.x86_64.rpm"},
            {"nevra":"lg-buddy-1.0.0-1.x86_64","action":"Replaced","reason":"User","repo_id":"@System"}
        ]})
    }
    fn verify(value: &serde_json::Value) -> Result<String, String> {
        verify_plan(
            &serde_json::to_vec(value).unwrap(),
            "1.0.0-1",
            "2.0.0-1",
            "lg-buddy-2.0.0-1.x86_64.rpm",
        )
        .map(|plan| plan.local_package)
    }
    #[test]
    fn prerelease_plan_checks_native_identity_and_portable_filename_separately() {
        let mut value = plan();
        value["rpms"][0]["nevra"] = json!("lg-buddy-2.0.0~beta.3-1.x86_64");
        value["rpms"][0]["package_path"] = json!("./packages/lg-buddy-2.0.0-beta.3-1.x86_64.rpm");
        let verify = |value: &serde_json::Value| {
            verify_plan(
                &serde_json::to_vec(value).unwrap(),
                "1.0.0-1",
                "2.0.0~beta.3-1",
                "lg-buddy-2.0.0-beta.3-1.x86_64.rpm",
            )
        };
        assert_eq!(
            verify(&value).unwrap().local_package,
            "./packages/lg-buddy-2.0.0-beta.3-1.x86_64.rpm"
        );
        value["rpms"][0]["package_path"] = json!("./packages/lg-buddy-2.0.0~beta.3-1.x86_64.rpm");
        assert!(verify(&value).is_err());
        value["rpms"][0]["package_path"] = json!("./packages/lg-buddy-2.0.0-beta.3-1.x86_64.rpm");
        value["rpms"][0]["nevra"] = json!("lg-buddy-2.0.0-1.x86_64");
        assert!(verify(&value).is_err());
    }
    #[test]
    fn exact_upgrade_plan_allows_repository_dependencies() {
        let mut value = plan();
        assert!(verify(&value).is_ok());
        value["rpms"].as_array_mut().unwrap().push(json!({"nevra":"example-1.0-1.x86_64","action":"Install","reason":"Dependency","repo_id":"fedora","package_path":"./packages/example-1.0-1.x86_64.rpm"}));
        assert!(verify(&value).is_ok());
    }
    #[test]
    fn stored_repository_ids_preserve_the_local_package_boundary() {
        let mut value = plan();
        value["rpms"][0]["repo_id"] = json!("@stored_transaction(@commandline)");
        value["rpms"].as_array_mut().unwrap().push(json!({
            "nevra":"example-1.0-1.x86_64", "action":"Install", "reason":"Dependency",
            "repo_id":"@stored_transaction(fedora)", "package_path":"./packages/example.rpm"
        }));
        let checked = verify_plan(
            &serde_json::to_vec(&value).unwrap(),
            "1.0.0-1",
            "2.0.0-1",
            "lg-buddy-2.0.0-1.x86_64.rpm",
        )
        .unwrap();
        assert_eq!(
            checked.local_package,
            "./packages/lg-buddy-2.0.0-1.x86_64.rpm"
        );
        assert_eq!(checked.repository_packages, ["./packages/example.rpm"]);
        for repository in [
            "@commandline",
            "@stored_transaction(@commandline)",
            "@System",
            "@stored_transaction(@System)",
            "@stored_transaction()",
            "@stored_transaction(fedora",
            "@stored_transaction(fedora))",
            "@stored_transaction(@stored_transaction(fedora))",
        ] {
            value["rpms"][2]["repo_id"] = json!(repository);
            assert!(
                verify(&value).is_err(),
                "accepted dependency from {repository}"
            );
        }
        value["rpms"][2]["repo_id"] = json!("@stored_transaction(fedora)");
        for repository in [
            "@stored_transaction(fedora)",
            "@stored_transaction(@commandline))",
        ] {
            value["rpms"][0]["repo_id"] = json!(repository);
            assert!(
                verify(&value).is_err(),
                "accepted LG Buddy from {repository}"
            );
        }
        value["rpms"][0]["repo_id"] = json!("@stored_transaction(@commandline)");
        value["rpms"][1]["repo_id"] = json!("@stored_transaction(@System)");
        assert!(verify(&value).is_err());
    }

    #[test]
    fn stored_repository_rpms_require_signatures_and_digests() {
        // The Fedora integration supplies genuine signed and unsigned RPMs;
        // ordinary unit runs do not require RPM tools or a system keyring.
        let Some(signed) = std::env::var_os("LG_BUDDY_TEST_SIGNED_RPM") else {
            return;
        };
        let unsigned = std::env::var_os("LG_BUDDY_TEST_UNSIGNED_RPM").unwrap();
        let temp = std::env::temp_dir().join(format!(
            "lg-buddy-rpm-signatures-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&temp).unwrap();
        fs::create_dir(temp.join("packages")).unwrap();
        let path = temp.join("packages/dependency.rpm");
        let plan = VerifiedPlan {
            local_package: "unused".into(),
            repository_packages: vec!["./packages/dependency.rpm".into()],
        };
        fs::copy(signed, &path).unwrap();
        verify_repository_packages(&temp, &plan).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(verify_repository_packages(&temp, &plan).is_err());
        fs::copy(unsigned, &path).unwrap();
        assert!(verify_repository_packages(&temp, &plan).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
        assert!(verify_repository_packages(&temp, &plan).is_err());
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn plan_refuses_destructive_actions_and_changed_identity() {
        for action in [
            "Remove",
            "Downgrade",
            "Reinstall",
            "Reason Change",
            "unknown",
        ] {
            let mut value = plan();
            value["rpms"][0]["action"] = json!(action);
            assert!(verify(&value).is_err());
        }
        for (field, content) in [
            ("nevra", "lg-buddy-2.0.0-1.aarch64"),
            ("repo_id", "fedora"),
            ("package_path", "./packages/../../evil.rpm"),
        ] {
            let mut value = plan();
            value["rpms"][0][field] = json!(content);
            assert!(verify(&value).is_err());
        }
        let mut value = plan();
        value["rpms"][1]["nevra"] = json!("foreign-1.0.0-1.x86_64");
        assert!(verify(&value).is_err());
        let mut value = plan();
        value["groups"] = json!([]);
        assert!(verify(&value).is_err());
        let mut value = plan();
        value["version"] = json!("2.0");
        assert!(verify(&value).is_err());
        let mut value = plan();
        value["rpms"] = json!([]);
        assert!(verify(&value).is_err());
    }
    #[test]
    fn installed_state_refuses_duplicate_epochs_and_noncanonical_versions() {
        assert_eq!(
            parse_installed_state("lg-buddy\n0\n2.0.0~beta.2.ci-1\nx86_64\n").unwrap(),
            "2.0.0~beta.2.ci-1"
        );
        for text in [
            "lg-buddy\n1\n2.0.0-1\nx86_64\n",
            "lg-buddy\n0\n2.0.0-1\naarch64\n",
            "lg-buddy\n0\n2.0.0~alpha.ci-1\nx86_64\n",
            "lg-buddy\n0\n2.0.0-01\nx86_64\n",
            "lg-buddy\n0\n2.0.0-1\nx86_64\nlg-buddy\n0\n2.0.0-2\nx86_64\n",
        ] {
            assert!(parse_installed_state(text).is_err());
        }
    }
    #[test]
    fn commands_retain_native_safety_and_repository_trust() {
        let command = store_command(
            Path::new("/root/private/package.rpm"),
            Path::new("/root/private/transaction"),
        );
        let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
        assert!(args.contains(&"--no-allow-downgrade"));
        assert!(args.contains(&"--setopt=*.gpgcheck=True"));
        for weakening in [
            "--nogpgcheck",
            "--allowerasing",
            "--skip-broken",
            "--skip-unavailable",
        ] {
            assert!(!args.contains(&weakening));
        }
        let command = replay_command(Path::new("/root/private/transaction"));
        let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(
            &args[args.len() - 2..],
            ["replay", "/root/private/transaction"]
        );
        assert!(!args.iter().any(|arg| arg.starts_with("--ignore-")));
    }
}
