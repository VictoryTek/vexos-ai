#!/usr/bin/env bash
# ev.sh 'js': Eval in the nested shell only (its private bus).
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
exec env -i PATH=/run/current-system/sw/bin DBUS_SESSION_BUS_ADDRESS="$(cat "$RUN/bus")" \
  gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
  --method org.gnome.Shell.Eval "$1"
