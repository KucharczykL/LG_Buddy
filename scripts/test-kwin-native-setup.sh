#!/bin/bash
# Run only in a disposable distro container with an ordinary user, passwordless
# sudo, KWin and its development files. Exercise setup without QT_PLUGIN_PATH.
set -euo pipefail
[ "${LG_BUDDY_DISPOSABLE_TEST:-}" = 1 ]
[ "$(id -u)" -ne 0 ]
runtime="${1:?installed runtime required}"
payload="${2:?installed KWin payload required}"
sudo -n true
export XDG_RUNTIME_DIR="$(mktemp -d)"
chmod 700 "$XDG_RUNTIME_DIR"
export HOME="$XDG_RUNTIME_DIR/home"
export XDG_CONFIG_HOME="$HOME/config" XDG_CACHE_HOME="$HOME/cache" XDG_DATA_HOME="$HOME/data" XDG_STATE_HOME="$HOME/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"
cleanup() {
    "$runtime" kwin-setup --payload-dir "$payload" --remove || true
    [ -z "${legacy_file:-}" ] || sudo -n rm -f -- "$legacy_file"
    if [ -n "${kwin_pid:-}" ]; then
        kill "$kwin_pid" 2>/dev/null || true
        wait "$kwin_pid" 2>/dev/null || true
    fi
    [ ! -f "$XDG_RUNTIME_DIR/kwin.log" ] || cat "$XDG_RUNTIME_DIR/kwin.log"
    [ ! -f "$XDG_STATE_HOME/lg-buddy/kwin/setup.log" ] || cat "$XDG_STATE_HOME/lg-buddy/kwin/setup.log"
    rm -rf -- "$XDG_RUNTIME_DIR"
}
trap cleanup EXIT
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET QT_QPA_PLATFORM QT_PLUGIN_PATH
export KWIN_COMPOSE=O2 LIBGL_ALWAYS_SOFTWARE=1
cp /usr/bin/kwin_wayland "$XDG_RUNTIME_DIR/kwin_wayland"
"$XDG_RUNTIME_DIR/kwin_wayland" --virtual --no-lockscreen --no-global-shortcuts > "$XDG_RUNTIME_DIR/kwin.log" 2>&1 &
kwin_pid=$!
ready=0
for _ in {1..150}; do
    kill -0 "$kwin_pid" 2>/dev/null || break
    if "$runtime" kwin-bridge info > "$XDG_RUNTIME_DIR/info" 2>/dev/null && [ -s "$XDG_RUNTIME_DIR/info" ]; then
        ready=1
        break
    fi
    sleep 0.1
done
[ "$ready" = 1 ]
cat "$XDG_RUNTIME_DIR/info"
status=0
"$runtime" kwin-setup --payload-dir "$payload" --status || status=$?
[ "$status" = 3 ]

"$runtime" kwin-setup --payload-dir "$payload" --foreground
"$runtime" kwin-bridge check
"$runtime" kwin-setup --payload-dir "$payload" --status
"$runtime" kwin-setup --payload-dir "$payload" --foreground
receipts="$XDG_STATE_HOME/lg-buddy/kwin/plugins"
[ "$(find "$receipts" -name '*.tsv' | wc -l)" = 1 ]
receipt="$(find "$receipts" -name '*.tsv' -print -quit)"
IFS=$'\t' read -r directory installed_id < "$receipt"
[ "$(stat -c '%u:%a' "$directory/kwin/plugins/$installed_id.so")" = 0:644 ]

"$runtime" kwin-setup --payload-dir "$payload" --remove
! "$runtime" kwin-bridge check
[ ! -e "$directory/kwin/plugins/$installed_id.so" ]

# A stale artifact and receipt simulate integration needing repair after an
# update. Keep the old lib64 spelling used by existing Arch installations.
legacy_id="lg_buddy_inhibition_$(id -u)_$(printf previous | sha256sum | cut -d ' ' -f1)"
legacy_root="$directory"
case "$(readlink /usr/lib64 || true)" in
    lib|/usr/lib) legacy_root=/usr/lib64/qt6/plugins ;;
esac
legacy_file="$legacy_root/kwin/plugins/$legacy_id.so"
printf previous | sudo -n tee "$legacy_file" >/dev/null
printf '%s\t%s\n' "$legacy_root" "$legacy_id" > "$receipts/$legacy_id.tsv"
"$runtime" kwin-setup --payload-dir "$payload" --foreground
"$runtime" kwin-bridge check
[ ! -e "$legacy_file" ]
[ "$(find "$receipts" -name '*.tsv' | wc -l)" = 1 ]
"$runtime" kwin-setup --payload-dir "$payload" --remove
! "$runtime" kwin-bridge check
[ "$(find "$receipts" -name '*.tsv' | wc -l)" = 0 ]
[ ! -e "$directory/kwin/plugins/$installed_id.so" ]
kill -0 "$kwin_pid"
echo 'PASS: native Plasma setup, repeat setup, legacy-receipt repair and cleanup with the real KWin loader.'
