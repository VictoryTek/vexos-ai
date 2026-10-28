#!/usr/bin/env bats
# The account commands and the zenity accounts panel are removed
# (docs/contract.md §Commands): Claude always uses its default sign-in. Every
# form exits 2 with one line on stderr, prints nothing on stdout, opens
# nothing, and leaves what older versions left on disk exactly as it was.

load helpers
setup() { common_setup; }

REMOVED=(
  "account" "account list" "account list --json" "account add" "account add work"
  "account use" "account use work" "account use next" "account use main"
  "account remove work" "account rename work office" "account mode"
  "account mode auto 90" "account mode manual ''" "account frobnicate"
  "--json account use main" "--non-interactive account add work" "panel"
)

# What an older version may have left: an extra account's config dir, the
# pointer to it, and the switching settings.
leftovers() {
  make_account work
  touch "$DATA/work/marker"
  set_file "$CONF/claude-account" work
  set_file "$CONF/mode" auto
  set_file "$CONF/threshold" 80
}

snapshot() { find "$HOME" -exec stat -c '%n %s %Y' {} + | sort; }

@test "every account command, and panel, exits 2 with one clear line on stderr" {
  leftovers
  install_agents
  export WAYLAND_DISPLAY=wayland-0
  local args
  for args in "${REMOVED[@]}"; do
    eval "run --separate-stderr vexos $args"
    [ "$status" -eq 2 ]
    [ -z "$output" ]
    [ "${#stderr_lines[@]}" -eq 1 ]
    [[ $stderr == "vexos-ai: '"*"' was removed: "*"default sign-in"* ]]
  done
  nothing_opened
  never_called claude.log
}

@test "on a terminal they exit 2 too, and start no sign-in" {
  skip_without_pty
  install_agents
  WAYLAND_DISPLAY=wayland-0 run_tty account add work
  [ "$status" -eq 2 ]
  WAYLAND_DISPLAY=wayland-0 run_tty panel
  [ "$status" -eq 2 ]
  nothing_opened
  never_called claude.log
  [ ! -e "$DATA/work" ]
}

@test "they leave old account dirs, the pointer and the settings untouched" {
  leftovers
  local before args
  before=$(snapshot)
  for args in "${REMOVED[@]}"; do eval "run vexos $args"; done
  [ "$(snapshot)" = "$before" ]
  [ -e "$DATA/work/marker" ]
  [ "$(cat "$CONF/claude-account")" = work ]
}

@test "--help offers no account command and no panel" {
  run vexos --help
  [ "$status" -eq 0 ]
  [[ $output != *"account"* ]]
  [[ $output != *"panel"* ]]
  [[ $output != *"5 busy"* ]]
}
