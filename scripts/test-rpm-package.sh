#!/bin/bash
# Uses real DNF/RPM and replaces service-manager commands in a disposable
# container. Offline enable/disable uses real systemctl; running-manager,
# session and reboot qualification belongs to the VM lane.
set -euo pipefail

[ "${LG_BUDDY_DISPOSABLE_PACKAGE_TEST:-}" = 1 ] && [ -f /.dockerenv ] && [ "$EUID" = 0 ] || {
    echo "Run only as root in a disposable Docker container with LG_BUDDY_DISPOSABLE_PACKAGE_TEST=1." >&2
    exit 1
}
[ "$#" = 2 ] || { echo "Usage: $0 <baseline.rpm> <newer-package.rpm>" >&2; exit 1; }
baseline=$(realpath "$1")
candidate=$(realpath "$2")
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
work=$(mktemp -d)
chmod 755 "$work"
export LC_ALL=C

cp -p /usr/bin/systemctl "$work/systemctl"
cleanup() {
    cp -p "$work/systemctl" /usr/bin/systemctl
    rm -rf -- "$work"
}
trap cleanup EXIT
# DNF calls clear tsflags so minimal-image nodocs defaults cannot hide payload.
mkdir -p /run/systemd/system
for command in systemctl; do
    cat >"/usr/bin/$command" <<EOF
#!/bin/sh
printf '%s\\n' "$command \$*" >> "$work/service-actions"
EOF
    chmod 755 "/usr/bin/$command"
done
touch "$work/service-actions"

fail() { echo "$*" >&2; exit 1; }
install_package() {
    if ! dnf5 install -y --setopt=install_weak_deps=False --setopt=tsflags= "$1" >"$work/dnf.log" 2>&1; then
        cat "$work/dnf.log" >&2
        fail "DNF installation failed"
    fi
    if grep -Fq "can't create transaction lock" "$work/dnf.log"; then
        cat "$work/dnf.log" >&2
        fail "RPM ownership queries attempted a nested transaction lock"
    fi
}
refuses_unpack() {
    if rpm --upgrade "$candidate" >"$work/refusal.log" 2>&1; then
        fail "RPM accepted a conflicting installation"
    fi
    grep -Fq "$1" "$work/refusal.log" || { cat "$work/refusal.log" >&2; fail "Missing conflict diagnostic"; }
}
no_activation() {
    if grep -E '(start|enable|restart|preset).*LG_Buddy' "$work/service-actions"; then
        fail "Package installation activated services"
    fi
}

python3 - "$baseline" "$candidate" <<'PYRPM'
import rpm, subprocess, sys
versions = [tuple(subprocess.check_output(["rpm", "-qp", "--qf", "%{EPOCHNUM}\n%{VERSION}\n%{RELEASE}\n", package], text=True).splitlines()) for package in sys.argv[1:]]
assert rpm.labelCompare(versions[0], versions[1]) < 0
PYRPM
[ "$(rpm -qp --qf '%{NAME}' "$candidate")" = lg-buddy ]
[ "$(rpm -qp --qf '%{ARCH}' "$candidate")" = x86_64 ]
depends=$(rpm -qp --requires "$candidate")
grep -Fxq 'gtk4 >= 4.14' <<<"$depends"
grep -Fxq 'libadwaita >= 1.5' <<<"$depends"
grep -Fxq 'libglvnd-gles' <<<"$depends"
if grep -Eiq 'python|bscpylgtv|zenity|swayidle|kwin|plasma|cmake|g\+\+|libQt|libKF' <<<"$depends"; then
    fail "Package requires obsolete or optional integration/build dependencies"
fi

# RPM normally adopts unowned files. The package must refuse before doing so.
printf '%s\n' 'unowned bundle sentinel' >/usr/bin/lg-buddy
refuses_unpack 'unowned or unverifiable payload /usr/bin/lg-buddy'
grep -Fxq 'unowned bundle sentinel' /usr/bin/lg-buddy
rm /usr/bin/lg-buddy
mkdir "$work/redirect"
ln -s "$work/redirect" /usr/lib/lg-buddy
refuses_unpack 'redirected directory /usr/lib/lg-buddy'
[ -z "$(ls -A "$work/redirect")" ]
rm /usr/lib/lg-buddy
mkdir -p /etc/systemd/system
printf '%s\n' 'local unit sentinel' >/etc/systemd/system/LG_Buddy.service
refuses_unpack 'local integration or legacy binding /etc/systemd/system/LG_Buddy.service'
grep -Fxq 'local unit sentinel' /etc/systemd/system/LG_Buddy.service
rm /etc/systemd/system/LG_Buddy.service
mkdir -p /usr/lib/systemd/system-sleep
printf '%s\n' 'legacy hook sentinel' >/usr/lib/systemd/system-sleep/LG_Buddy_sleep_hook
refuses_unpack 'local integration or legacy binding /usr/lib/systemd/system-sleep/LG_Buddy_sleep_hook'
grep -Fxq 'legacy hook sentinel' /usr/lib/systemd/system-sleep/LG_Buddy_sleep_hook
rm /usr/lib/systemd/system-sleep/LG_Buddy_sleep_hook

# RPM otherwise permits identical files to belong to multiple packages. Prove
# that the guard refuses even an identical helper owned by another package.
rpm2archive -n - <"$candidate" | tar -xOf - ./usr/lib/lg-buddy/common.sh >"$work/foreign-common.sh"
cat >"$work/foreign.spec" <<EOF
%global debug_package %{nil}
%global __os_install_post %{nil}
Name: lg-buddy-foreign-fixture
Version: 1
Release: 1
Summary: Disposable overlapping ownership fixture
License: GPL-3.0-only
BuildArch: noarch
%description
Disposable overlapping ownership fixture.
%install
mkdir -p "%{buildroot}/usr/lib/lg-buddy"
install -m 755 "$work/foreign-common.sh" "%{buildroot}/usr/lib/lg-buddy/common.sh"
%files
%defattr(-,root,root,-)
/usr/lib/lg-buddy/common.sh
EOF
rpmbuild -bb --define "_topdir $work/foreign" "$work/foreign.spec" >"$work/foreign-build.log" 2>&1
rpm --install "$work/foreign/RPMS/noarch/lg-buddy-foreign-fixture-1-1.noarch.rpm"
refuses_unpack 'conflicting payload /usr/lib/lg-buddy/common.sh'
[ "$(rpm -qf --qf '%{NAME}' /usr/lib/lg-buddy/common.sh)" = lg-buddy-foreign-fixture ]
cmp "$work/foreign-common.sh" /usr/lib/lg-buddy/common.sh
rpm --erase lg-buddy-foreign-fixture
rmdir /usr/lib/lg-buddy

# Root installation must leave both current and stale user data untouched.
useradd --create-home --shell /bin/bash lgrpm-migrate
mkdir -p "$work/current/tvs/primary" "$work/stale/tvs/primary"
printf '%s\n' 'tvs_primary_platform=lg_webos' '# retained current settings' >"$work/current/config.env"
cat >"$work/stale/config.env" <<'EOF'
# retained stale settings
tvs_primary_ip=127.0.0.2
tvs_primary_mac=02:00:00:00:01:34
tvs_primary_input=HDMI_3
tvs_primary_platform=bscpylgtv
screen_backend=swayidle
screen_idle_blank=disabled
screen_idle_timeout=37
screen_restore_policy=aggressive
updates_channel=prerelease
updates_auto_check=disabled
system_sleep_wake_policy=disabled
EOF
printf '%s\n' '{"access_token":"retained-current-token"}' >"$work/current/tvs/primary/access-token.json"
printf '%s\n' '{"access_token":"retained-stale-token"}' >"$work/stale/tvs/primary/access-token.json"
chown -R 1000:1000 "$work/current"
chown -R "$(id -u lgrpm-migrate):$(id -g lgrpm-migrate)" "$work/stale"
chmod 640 "$work/stale/config.env"
chmod 600 "$work/"{current,stale}/tvs/primary/access-token.json
mkdir -p /etc/lg-buddy /etc/systemd/system/LG_Buddy_lifecycle.service.d
printf '%s\n' "$work/current/config.env" >/etc/lg-buddy/config-path
printf '[Service]\nEnvironment="LG_BUDDY_CONFIG=%s/current/config.env"\n' "$work" \
    >/etc/systemd/system/LG_Buddy_lifecycle.service.d/config.conf
retained=("$work/current" "$work/stale" /etc/lg-buddy/config-path /etc/systemd/system/LG_Buddy_lifecycle.service.d/config.conf)
snapshot() {
    find "${retained[@]}" -type f -print0 | sort -z | xargs -0 sha256sum
    find "${retained[@]}" -printf '%p %U %G %m\n' | sort
}
snapshot >"$work/before"
preserved() { snapshot >"$work/after"; cmp "$work/before" "$work/after"; }

check_installed() {
    local package=$1
    [ "$(rpm -q --qf '%{VERSION}-%{RELEASE}' lg-buddy)" = "$(rpm -qp --qf '%{VERSION}-%{RELEASE}' "$package")" ]
    [ "$(rpm -qf --qf '%{NAME}' /usr/bin/lg-buddy)" = lg-buddy ]
    rpm -V lg-buddy >"$work/verify.log"
    python3 - <<'PY'
import json, pathlib, stat, subprocess
manifest = json.loads(pathlib.Path('/usr/share/doc/lg-buddy/release-manifest.json').read_text())
expected = f'lg-buddy {manifest["version"]}\nversion: {manifest["version"]}\nchannel: {manifest["channel"]}\ncommit: {manifest["commit"]}\n'
for binary in ('lg-buddy', 'lg-buddy-gui'):
    assert subprocess.check_output(['/usr/bin/' + binary, '--version'], text=True) == expected
files = subprocess.check_output(['rpm', '-ql', 'lg-buddy'], text=True).splitlines()
for name in files:
    assert not name.startswith(('/etc/', '/home/', '/var/', '/run/')), name
    path = pathlib.Path(name)
    metadata = path.lstat()
    assert metadata.st_uid == metadata.st_gid == 0, name
    assert not path.is_symlink(), name
    if path.is_file():
        expected_mode = 0o755 if path.name in ('lg-buddy', 'lg-buddy-gui', 'setup-services', 'setup.sh', 'build.sh', 'common.sh', 'LG_Buddy_lifecycle') else 0o644
        assert stat.S_IMODE(metadata.st_mode) == expected_mode, name
PY
    /usr/bin/lg-buddy-gui --check-runtime
    bash "$script_dir/test-release-linkage.sh" /usr/bin/lg-buddy /usr/bin/lg-buddy-gui
    preserved
    no_activation
}

install_package "$baseline"
check_installed "$baseline"
install_package "$candidate"
check_installed "$candidate"
dnf5 reinstall -y --setopt=install_weak_deps=False --setopt=tsflags= "$candidate" >"$work/reinstall.log" 2>&1
check_installed "$candidate"

# Exercise the installed executable, without a desktop, session bus or TV.
# Root package operations above preserved the complete stale input; only its
# owner's screen-daemon entrypoint may now convert it.
python3 "$script_dir/test-package-startup-migration.py" "$work/stale/config.env" lgrpm-migrate "$work/migration-monitor.log"
# The deliberate user conversion is now the retention baseline for removal.
snapshot >"$work/before"

useradd --create-home --shell /bin/bash lgrpm-gui
runuser -u lgrpm-gui -- dbus-run-session -- xvfb-run -a \
    bash "$script_dir/test-gui-launch.sh" /usr/bin/lg-buddy /usr/bin/lg-buddy-gui
preserved

dnf5 remove -y --setopt=clean_requirements_on_remove=False lg-buddy >"$work/remove.log" 2>&1
grep -Fxq 'systemctl --system stop LG_Buddy_lifecycle.service LG_Buddy.service' "$work/service-actions"
grep -Fxq 'systemctl --system --no-reload disable LG_Buddy_lifecycle.service LG_Buddy.service' "$work/service-actions"
[ ! -e /usr/bin/lg-buddy ] && [ ! -e /usr/bin/lg-buddy-gui ]
[ ! -e /usr/lib/lg-buddy/setup-services ]
preserved

# Native offline image removal must also remove enablement links. No manager
# or LG Buddy process runs here; systemctl changes only the unit symlinks.
cp -p "$work/systemctl" /usr/bin/systemctl
rmdir /run/systemd/system
install_package "$candidate"
systemctl --system --no-reload enable LG_Buddy_lifecycle.service
enabled=/etc/systemd/system/multi-user.target.wants/LG_Buddy_lifecycle.service
[ -L "$enabled" ]
dnf5 remove -y --setopt=clean_requirements_on_remove=False lg-buddy >"$work/offline-remove.log" 2>&1
[ ! -L "$enabled" ]
preserved
echo 'RPM package lifecycle passed: conflicts, install, upgrade, reinstall, GUI, online/offline removal and retention.'
