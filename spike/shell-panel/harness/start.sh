#!/usr/bin/env bash
# Starts a headless nested GNOME Shell (1280x800) for the spike, with the spike
# extension and a test-only helper that allows Eval and Screenshot, all inside
# target/shell-spike-run/ on a private session bus. Returns when the shell
# answers Eval. Stop it with stop.sh.
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

compile=$(command -v glib-compile-schemas || true)
if [ -z "$compile" ]; then
  compile=$(find /nix/store -maxdepth 3 -path '*-glib-2*-dev/bin/glib-compile-schemas' 2>/dev/null | head -1)
fi
[ -n "$compile" ] || { echo "glib-compile-schemas not found" >&2; exit 1; }

mkdir -p "$RUN"/{home,config,data/gnome-shell/extensions,cache,state,run,tmp,shots}
chmod 700 "$RUN/run"
ext=$RUN/data/gnome-shell/extensions
mkdir -p "$ext/$UUID" "$ext/unsafe-test@spikerun"
cp -rT "$EXT_SRC" "$ext/$UUID"
"$compile" "$ext/$UUID/schemas"

cat > "$ext/unsafe-test@spikerun/metadata.json" <<'JSON'
{"uuid": "unsafe-test@spikerun", "name": "spike unsafe mode", "description": "Test only: lets the nested shell answer Eval and Screenshot.", "shell-version": ["50"], "url": "https://example.invalid"}
JSON
cat > "$ext/unsafe-test@spikerun/extension.js" <<'JS'
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
export default class Unsafe extends Extension {
    enable() { global.context.unsafe_mode = true; }
    disable() { global.context.unsafe_mode = false; }
}
JS

for kind in session system; do
  cat > "$RUN/$kind.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>$kind</type>
  <listen>unix:dir=$RUN/run</listen>
  <servicedir>$RUN/data/dbus-1/services</servicedir>
  <servicedir>/run/current-system/sw/share/dbus-1/services</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
done

# A shell that was killed leaves this crash flag behind, and the next start
# then runs in safe mode with every extension off.
rm -f "$RUN/run/gnome-shell-disable-extensions"

# GNOME Shell 50 refuses to start without a system bus: an empty stand-in.
# It runs in the nested session's environment (XDG_RUNTIME_DIR names this run),
# so stop.sh finds it like every other process of the run.
sysbus=$(env -i PATH=/run/current-system/sw/bin HOME="$RUN/home" XDG_RUNTIME_DIR="$RUN/run" \
  dbus-daemon --config-file="$RUN/system.conf" --fork --print-address=1)

# The inner script runs under env -i, so it reads RUN and UUID from its environment.
# shellcheck disable=SC2016
nohup env -i \
  PATH=/run/current-system/sw/bin \
  HOME="$RUN/home" XDG_CONFIG_HOME="$RUN/config" XDG_DATA_HOME="$RUN/data" XDG_CACHE_HOME="$RUN/cache" \
  XDG_STATE_HOME="$RUN/state" XDG_RUNTIME_DIR="$RUN/run" TMPDIR="$RUN/tmp" \
  XDG_DATA_DIRS="$RUN/data:${XDG_DATA_DIRS:-}:/run/current-system/sw/share" \
  NIX_GSETTINGS_OVERRIDES_DIR="${NIX_GSETTINGS_OVERRIDES_DIR:-}" \
  XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME \
  DBUS_SYSTEM_BUS_ADDRESS="$sysbus" SHELL_DEBUG=backtrace-warnings RUN="$RUN" UUID="$UUID" \
  dbus-run-session --config-file="$RUN/session.conf" -- bash -c '
    echo "$DBUS_SESSION_BUS_ADDRESS" > "$RUN/bus"
    gsettings set org.gnome.shell welcome-dialog-last-shown-version "999"
    exec gnome-shell --headless --wayland --no-x11 --virtual-monitor 1280x800' \
  > "$RUN/shell.log" 2>&1 &

here=$(dirname "${BASH_SOURCE[0]}")
ext_call() {
  env -i PATH=/run/current-system/sw/bin DBUS_SESSION_BUS_ADDRESS="$(cat "$RUN/bus")" \
    gdbus call --session --dest org.gnome.Shell.Extensions --object-path /org/gnome/Shell/Extensions \
    --method "org.gnome.Shell.Extensions.$1" "$2"
}

# Enabling through the shell's own D-Bus API is reliable; the settings key is
# not (the shell may start before it is written).
for _ in $(seq 1 60); do
  if [ -s "$RUN/bus" ] && ext_call EnableExtension unsafe-test@spikerun >/dev/null 2>&1; then
    sleep 1
    if "$here/ev.sh" 'global.context.unsafe_mode' 2>/dev/null | grep -q '^(true'; then
      # Only the spike runs in the nested shell: switch off the host's other
      # extensions that its data dirs make visible (system defaults enable
      # them). Twice, since some are still loading the first time.
      for _pass in 1 2; do
        "$here/ev.sh" "Main.extensionManager.getUuids().filter(u => u !== 'unsafe-test@spikerun' && u !== '$UUID' && Main.extensionManager.lookup(u).state === 1).map(u => { Main.extensionManager.disableExtension(u); return u; }).length" >/dev/null
        sleep 2
      done
      ext_call EnableExtension "$UUID" >/dev/null
      # A fresh shell starts in the overview (a modal grab); tests start from the desktop.
      "$here/ev.sh" 'Main.overview.hide()' >/dev/null
      sleep 2
      echo "nested shell ready; log: $RUN/shell.log"
      exit 0
    fi
  fi
  sleep 1
done
echo "nested shell did not answer Eval; see $RUN/shell.log" >&2
exit 1
