#!/usr/bin/env bash
# shot.sh name: a screenshot of the nested shell into target/shell-spike-run/shots/
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
exec env -i PATH=/run/current-system/sw/bin DBUS_SESSION_BUS_ADDRESS="$(cat "$RUN/bus")" \
  gdbus call --session --dest org.gnome.Shell.Screenshot --object-path /org/gnome/Shell/Screenshot \
  --method org.gnome.Shell.Screenshot.Screenshot false false "$RUN/shots/$1.png"
