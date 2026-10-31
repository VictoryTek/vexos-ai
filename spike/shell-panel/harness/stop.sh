#!/usr/bin/env bash
# Stops only processes of the nested session: those whose environment has this
# run's XDG_RUNTIME_DIR. Never matches by command line or name.
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
for p in $(pgrep -u "$(id -u)"); do
  if { tr '\0' '\n' < "/proc/$p/environ"; } 2>/dev/null | grep -qx "XDG_RUNTIME_DIR=$RUN/run"; then
    echo "stopping $p $(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null | cut -c1-60)"
    kill "$p" 2>/dev/null
  fi
done
true
