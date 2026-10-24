#!/bin/bash
# Real helper file operations; systemctl and installation root are isolated.
set -euo pipefail
repo="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
test_path="$PATH"
source "$repo/data/setup-services.sh"
PATH="$test_path"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
setup_root="$fixture/root"
payload="$repo/systemd"
config="$fixture/a % \"quoted\" config.env"
printf '%s\n' screen_idle_blank=enabled > "$config"
cp "$config" "$fixture/original.env"
mkdir -p "$fixture/enabled"
systemctl() {
    printf '%s\n' "$*" >> "$fixture/actions"
    case "$1" in
        show)
            if [ "${@: -1}" = LG_Buddy_lifecycle.service ]; then
                if [ -f "$fixture/lifecycle-loaded" ]; then printf 'loaded\n'; else printf 'not-found\n'; fi
            elif [ -f "$setup_root/etc/systemd/system/${@: -1}" ]; then
                printf 'loaded\n'
            else
                printf 'not-found\n'
            fi
            ;;
        is-enabled) [ -f "$fixture/enabled/$3" ] ;;
        enable) touch "$fixture/enabled/$2" ;;
        disable) rm -f "$fixture/enabled/${@: -1}" ;;
        stop)
            [ ! -f "$fixture/fail-stop" ] || return 1
            if [ "$2" = LG_Buddy_lifecycle.service ]; then
                rm -f "$fixture/lifecycle-active"
            else
                [ ! -f "$fixture/fail-legacy-stop" ] || return 1
                rm -f "$fixture/active-$2"
            fi
            ;;
        daemon-reload)
            [ ! -f "$fixture/fail-reload" ] || return 1
            cp "$setup_root/etc/systemd/system/LG_Buddy_lifecycle.service.d/config.conf" "$fixture/lifecycle-loaded"
            ;;
        restart)
            [ ! -f "$fixture/start-exit" ] || return "$(cat "$fixture/start-exit")"
            [ ! -f "$fixture/fail-start" ] || return 1
            cp "$fixture/lifecycle-loaded" "$fixture/lifecycle-running"
            touch "$fixture/lifecycle-active"
            ;;
        *) echo 'unexpected systemctl action' >&2; return 1 ;;
    esac
}
systemd-tmpfiles() {
    printf '%s\n' "$*" >> "$fixture/tmpfiles-actions"
    [ ! -f "$fixture/fail-after-reload" ]
}
package_owned() { [ -f "$fixture/package-owned" ]; }
service_setup_ownership() { printf '%s\n' unowned; }
# Migrate the retired suspend/wake rails before starting the shared lifecycle.
legacy_paths=(
    etc/systemd/system/LG_Buddy_wake.service
    etc/systemd/system/LG_Buddy_sleep.service
    etc/systemd/system/LG_Buddy_wake.service.d/config.conf
    etc/systemd/system/LG_Buddy_sleep.service.d/config.conf
    etc/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_sleep
    usr/lib/systemd/system-sleep/LG_Buddy_sleep_hook
)
seed_legacy() {
    for relative in "${legacy_paths[@]}"; do
        mkdir -p "$(dirname "$setup_root/$relative")"
        printf 'legacy\n' > "$setup_root/$relative"
    done
    for unit in LG_Buddy_wake.service LG_Buddy_sleep.service; do
        touch "$fixture/enabled/$unit" "$fixture/active-$unit"
    done
}
seed_legacy
repair_services "$config"
for relative in "${legacy_paths[@]}"; do
    [ ! -e "$setup_root/$relative" ] || { echo "Legacy handler retained: $relative" >&2; exit 1; }
done
for unit in LG_Buddy_wake.service LG_Buddy_sleep.service; do
    test ! -e "$fixture/enabled/$unit"
    test ! -e "$fixture/active-$unit"
done
for unit in LG_Buddy.service LG_Buddy_lifecycle.service; do
    cmp "$payload/$unit" "$setup_root/etc/systemd/system/$unit"
    test -f "$fixture/enabled/$unit"
done
test -f "$fixture/lifecycle-active"
cmp "$config" "$fixture/original.env"
cmp "$payload/lg_buddy.conf" "$setup_root/etc/tmpfiles.d/lg_buddy.conf"
test "$(cat "$setup_root/etc/lg-buddy/config-path")" = "$config"
test "$(stat -c %a "$setup_root/etc/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle")" = 755
python3 - "$config" "$setup_root/etc/systemd/system/LG_Buddy.service.d/config.conf" <<'PY'
import pathlib, sys
escaped = sys.argv[1].replace('\\', '\\\\').replace('"', '\\"').replace('%', '%%')
assert pathlib.Path(sys.argv[2]).read_text() == f'[Service]\nEnvironment="LG_BUDDY_CONFIG={escaped}"\n'
PY
# Reapplying leaves the files/inodes intact. Reconciliation of manager state
# remains intentional; the backend skips this helper for a complete setup.
before="$(find "$setup_root" -type f -printf '%p %i\n' | sort)"
: > "$fixture/actions"
repair_services "$config"
test "$before" = "$(find "$setup_root" -type f -printf '%p %i\n' | sort)"
! grep -q '^enable ' "$fixture/actions"
! grep -Eq '^(stop|disable) .*LG_Buddy_(wake|sleep)\.service' "$fixture/actions"
# Failed migration must retain the old files for retry and never report success.
seed_legacy
touch "$fixture/fail-legacy-stop"
if repair_services "$config"; then echo 'failed legacy stop reported success' >&2; exit 1; fi
for relative in "${legacy_paths[@]}"; do test -f "$setup_root/$relative"; done
test ! -f "$fixture/lifecycle-active"
rm "$fixture/fail-legacy-stop"
# Do not delete files that belong to a package, including legacy handlers.
touch "$fixture/package-owned"
if repair_services "$config"; then echo 'removed package-owned legacy files' >&2; exit 1; fi
for relative in "${legacy_paths[@]}"; do test -f "$setup_root/$relative"; done
rm "$fixture/package-owned"
repair_services "$config"
for relative in "${legacy_paths[@]}"; do test ! -e "$setup_root/$relative"; done
# Partial installation and interrupted manager reload recover on retry.
rm "$setup_root/etc/tmpfiles.d/lg_buddy.conf"
touch "$fixture/fail-reload"
: > "$fixture/actions"
if repair_services "$config"; then echo 'failed reload reported success' >&2; exit 1; fi
! grep -q '^restart ' "$fixture/actions"
rm "$fixture/fail-reload"
repair_services "$config"
cmp "$payload/lg_buddy.conf" "$setup_root/etc/tmpfiles.d/lg_buddy.conf"
# Interruption after reload must leave a stopped service, even though loaded
# properties already have the new binding. Retry starts a new process with it.
next_config="$fixture/next.env"
cp "$config" "$next_config"
touch "$fixture/fail-after-reload"
if repair_services "$next_config"; then echo 'interruption reported success' >&2; exit 1; fi
! cmp -s "$fixture/lifecycle-loaded" "$fixture/lifecycle-running"
test ! -f "$fixture/lifecycle-active"
rm "$fixture/fail-after-reload"
touch "$fixture/fail-start"
if repair_services "$next_config"; then echo 'failed start reported success' >&2; exit 1; fi
test ! -f "$fixture/lifecycle-active"
rm "$fixture/fail-start"
for code in 126 127; do
    printf '%s' "$code" > "$fixture/start-exit"
    if repair_services "$next_config"; then status=0; else status=$?; fi
    test "$status" = 1
done
rm "$fixture/start-exit"
repair_services "$next_config"
test -f "$fixture/lifecycle-active"
cmp "$fixture/lifecycle-loaded" "$fixture/lifecycle-running"
# Failure to stop must leave both disk and loaded configuration untouched.
touch "$fixture/fail-stop"
if repair_services "$config"; then echo 'failed stop reported success' >&2; exit 1; fi
test "$(cat "$setup_root/etc/lg-buddy/config-path")" = "$next_config"
cmp "$setup_root/etc/systemd/system/LG_Buddy_lifecycle.service.d/config.conf" "$fixture/lifecycle-running"
test -f "$fixture/lifecycle-active"
rm "$fixture/fail-stop"
repair_services "$config"
# Symlinks and package-owned modifications cannot be overwritten.
unit="$setup_root/etc/systemd/system/LG_Buddy.service"
rm "$unit"
ln -s "$config" "$unit"
if repair_services "$config"; then echo 'replaced symlink' >&2; exit 1; fi
cmp "$config" "$fixture/original.env"
rm "$unit"
printf 'managed elsewhere\n' > "$unit"
touch "$fixture/package-owned"
if repair_services "$config" 2>/dev/null; then echo 'replaced package file' >&2; exit 1; fi
grep -qx 'managed elsewhere' "$unit"
rm "$fixture/package-owned"
repair_services "$config"
# Package mode verifies all vendor payloads before even stopping a service,
# and writes only the machine-specific drop-ins. No /usr pointer is created.
setup_root="$fixture/package-root"
service_setup_ownership() { if [ "$#" -eq 0 ]; then printf '%s\n' package; else printf '%s\n' unowned; fi; }
for directory in system user; do
    mkdir -p "$setup_root/usr/lib/systemd/$directory"
done
for unit in LG_Buddy.service LG_Buddy_lifecycle.service; do
    install -m 644 "$payload/$unit" "$setup_root/usr/lib/systemd/system/$unit"
done
for unit in LG_Buddy_screen.service LG_Buddy_update_check.service LG_Buddy_update_check.timer; do
    install -m 644 "$payload/$unit" "$setup_root/usr/lib/systemd/user/$unit"
done
mkdir -p "$setup_root/usr/lib/tmpfiles.d" "$setup_root/usr/lib/NetworkManager/dispatcher.d/pre-down.d"
install -m 644 "$payload/lg_buddy.conf" "$setup_root/usr/lib/tmpfiles.d/lg_buddy.conf"
lifecycle_hook > "$setup_root/usr/lib/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle"
chmod 755 "$setup_root/usr/lib/NetworkManager/dispatcher.d/pre-down.d/LG_Buddy_lifecycle"
before="$(find "$setup_root/usr" -type f -printf '%p %i %T@ %m\n' | sort)"
repair_services "$config"
test "$before" = "$(find "$setup_root/usr" -type f -printf '%p %i %T@ %m\n' | sort)"
test -f "$setup_root/etc/systemd/system/LG_Buddy.service.d/config.conf"
test ! -e "$setup_root/etc/systemd/system/LG_Buddy.service"
test ! -e "$setup_root/etc/tmpfiles.d/lg_buddy.conf"
test ! -e "$setup_root/usr/lib/lg-buddy/config-path"
test "$(cat "$setup_root/etc/lg-buddy/config-path")" = "$config"
grep -Fx -- "--create $setup_root/usr/lib/tmpfiles.d/lg_buddy.conf" "$fixture/tmpfiles-actions"
# A package-owned drop-in is refused before stopping services or writing files.
service_setup_ownership() { printf '%s\n' package; }
: > "$fixture/actions"
if repair_services "$config"; then echo 'overwrote package-owned drop-in' >&2; exit 1; fi
test ! -s "$fixture/actions"
service_setup_ownership() { if [ "$#" -eq 0 ]; then printf '%s\n' package; else printf '%s\n' unowned; fi; }
# Symlinks in either drop-in directory are refused before the first service
# action; lexical ownership queries must never authorize a redirected write.
for unit in LG_Buddy.service LG_Buddy_lifecycle.service; do
    directory="$setup_root/etc/systemd/system/$unit.d"
    vendor="$setup_root/usr/lib/systemd/system/$unit.d"
    mv "$directory" "$fixture/dropin-backup"
    mkdir -p "$vendor"
    printf 'package default\n' > "$vendor/config.conf"
    ln -s "$vendor" "$directory"
    : > "$fixture/actions"
    if repair_services "$config"; then echo 'followed a package drop-in symlink' >&2; exit 1; fi
    test ! -s "$fixture/actions"
    if ownership=package write_if_changed "$directory/config.conf" 644 <<< 'redirected'; then
        echo 'writer followed a package drop-in symlink' >&2; exit 1
    fi
    grep -qx 'package default' "$vendor/config.conf"
    rm "$directory"
    mv "$fixture/dropin-backup" "$directory"
    rm "$vendor/config.conf"
    rmdir "$vendor"
done
# An earlier ancestor and a dangling drop-in symlink must also fail before
# creating anything at their targets, including missing paths under /usr/lib.
for variant in ancestor dangling; do
    if [ "$variant" = ancestor ]; then directory="$setup_root/etc/systemd"; else directory="$setup_root/etc/systemd/system/LG_Buddy.service.d"; fi
    mv "$directory" "$fixture/dropin-backup"
    vendor="$setup_root/usr/lib/redirected-$variant"
    if [ "$variant" = ancestor ]; then mkdir -p "$vendor"; fi
    ln -s "$vendor" "$directory"
    : > "$fixture/actions"
    if repair_services "$config"; then echo 'followed redirected package path' >&2; exit 1; fi
    test ! -s "$fixture/actions"
    test ! -e "$vendor/system"
    test ! -e "$vendor/config.conf"
    rm "$directory"
    mv "$fixture/dropin-backup" "$directory"
done
# Every missing vendor asset refuses before any file write or service action.
while IFS= read -r path; do
    mv "$path" "$fixture/removed-vendor"
    : > "$fixture/actions"
    before="$(find "$setup_root" -type f -printf '%p %i %T@\n' | sort)"
    if repair_services "$config"; then echo 'recreated missing package asset' >&2; exit 1; fi
    test ! -s "$fixture/actions"
    test "$before" = "$(find "$setup_root" -type f -printf '%p %i %T@\n' | sort)"
    mv "$fixture/removed-vendor" "$path"
done < <(find "$setup_root/usr" -type f)
# Probe failure, malformed output, and shadowing cannot select mutable repair.
service_setup_ownership() { return 1; }
: > "$fixture/actions"
if repair_services "$config"; then echo 'ignored package probe failure' >&2; exit 1; fi
test ! -s "$fixture/actions"
service_setup_ownership() { printf '%s\n' invalid; }
if repair_services "$config"; then echo 'ignored invalid ownership' >&2; exit 1; fi
test ! -s "$fixture/actions"
service_setup_ownership() { if [ "$#" -eq 0 ]; then printf '%s\n' package; else printf '%s\n' unowned; fi; }
printf 'local shadow\n' > "$setup_root/etc/systemd/system/LG_Buddy.service"
if repair_services "$config"; then echo 'ignored shadowing base unit' >&2; exit 1; fi
test ! -s "$fixture/actions"
rm "$setup_root/etc/systemd/system/LG_Buddy.service"
# The machine binding uses the same safe destination policy in both modes.
for binding_mode in unowned package; do
    service_setup_ownership() {
        if [ "$#" -eq 0 ]; then printf '%s\n' "$binding_mode"; else printf '%s\n' unowned; fi
    }
    binding_directory="$setup_root/etc/lg-buddy"
    mv "$binding_directory" "$fixture/binding-backup"
    mkdir -p "$fixture/redirected-binding"
    printf 'retained\n' > "$fixture/redirected-binding/config-path"
    ln -s "$fixture/redirected-binding" "$binding_directory"
    : > "$fixture/actions"
    if repair_services "$config"; then echo 'followed machine binding directory link' >&2; exit 1; fi
    test ! -s "$fixture/actions"
    test "$(cat "$fixture/redirected-binding/config-path")" = retained
    rm "$binding_directory"
    mv "$fixture/binding-backup" "$binding_directory"
    chmod 777 "$binding_directory"
    if repair_services "$config"; then echo 'accepted a writable binding directory' >&2; exit 1; fi
    test ! -s "$fixture/actions"
    chmod 755 "$binding_directory"
done

# Exercise the installed runtime's real ownership entry point against pinned
# package-tool paths, without querying or mutating this host's package database.
if [ "$#" -eq 1 ]; then
    runtime="$(readlink -f -- "$1")"
    # Keep the host loader/libraries available, including on merged-/usr
    # distributions. The probe interpreter lives outside the hidden bin directory.
    cp -L -- /bin/sh "$fixture/probe-sh"
    for verdict in package unowned conflicting failed absent; do
        printf '#!%s\n' "$fixture/probe-sh" > "$fixture/dpkg-query"
        cat >> "$fixture/dpkg-query" <<'PROBE'
case "$LG_BUDDY_TEST_VERDICT" in
    package) printf 'lg-buddy: %s\n' "$2" ;;
    unowned) exit 1 ;;
    conflicting) printf 'lg-buddy, other: %s\n' "$2" ;;
    *) exit 2 ;;
esac
PROBE
        chmod 755 "$fixture/dpkg-query"
        args=(--ro-bind / / --dev /dev --tmpfs /usr/bin --ro-bind "$runtime" /usr/bin/lg-buddy)
        if [ "$verdict" != absent ]; then args+=(--ro-bind "$fixture/dpkg-query" /usr/bin/dpkg-query); fi
        if LG_BUDDY_TEST_VERDICT="$verdict" bwrap "${args[@]}" /usr/bin/lg-buddy service-setup-ownership > "$fixture/ownership-output" 2> "$fixture/ownership-error"; then
            case "$verdict" in
                package|unowned) grep -qx "$verdict" "$fixture/ownership-output" ;;
                *) echo "unsafe ownership verdict accepted: $verdict" >&2; exit 1 ;;
            esac
        else
            case "$verdict" in package|unowned) cat "$fixture/ownership-error" >&2; exit 1 ;; esac
            test ! -s "$fixture/ownership-output"
            test -s "$fixture/ownership-error"
        fi
    done
fi
# Privilege/config validation never dispatches a repair with invalid inputs.
repair_services() { touch "$fixture/unsafe-main"; }
! main relative.env
! main "$config" unexpected
! main "$fixture/missing.env"
test ! -e "$fixture/unsafe-main"
echo 'PASS: service repair payloads, bindings, retries, ownership and unchanged-file preservation.'
