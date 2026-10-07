#!/usr/bin/env bats
# `vexos-ai crash-mute` and `crash-capture` — the ai-crash-mute recipe.

load helpers
setup() { common_setup; }

@test "crash-mute with no name lists the muted programs, one per line" {
  run vexos crash-mute
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  mkdir -p "$CONF/crash-ignore"
  touch "$CONF/crash-ignore/steam" "$CONF/crash-ignore/firefox"
  run vexos crash-mute
  [ "$status" -eq 0 ]
  [ "$output" = $'firefox\nsteam' ]
}

@test "empty name and state (just's defaults) list, and open nothing" {
  WAYLAND_DISPLAY=wayland-0 run vexos crash-mute "" ""
  [ "$status" -eq 0 ]
  nothing_opened
}

@test "crash-mute <name> mutes; with an empty state it is the same" {
  WAYLAND_DISPLAY=wayland-0 run vexos crash-mute firefox ""
  [ "$status" -eq 0 ]
  [ "$output" = "Crash notifications for 'firefox' are muted. Undo: vexos-ai crash-mute 'firefox' off" ]
  [ -e "$CONF/crash-ignore/firefox" ]
  nothing_opened
}

@test "crash-mute <name> on is the explicit form" {
  run vexos crash-mute steam on
  [ "$status" -eq 0 ]
  [ -e "$CONF/crash-ignore/steam" ]
}

@test "crash-mute <name> off unmutes" {
  mkdir -p "$CONF/crash-ignore"
  touch "$CONF/crash-ignore/firefox"
  run vexos crash-mute firefox off
  [ "$status" -eq 0 ]
  [ "$output" = "Crash notifications for 'firefox' are on again." ]
  [ ! -e "$CONF/crash-ignore/firefox" ]
}

@test "crash-mute --off <name> is the same as <name> off" {
  mkdir -p "$CONF/crash-ignore"
  touch "$CONF/crash-ignore/firefox"
  run vexos crash-mute --off firefox
  [ "$status" -eq 0 ]
  [ ! -e "$CONF/crash-ignore/firefox" ]
}

@test "crash-mute --off without a name is a usage error" {
  run vexos crash-mute --off
  [ "$status" -eq 2 ]
}

@test "crash-mute with a state that is neither on nor off is a usage error" {
  run vexos crash-mute firefox maybe
  [ "$status" -eq 2 ]
  [ ! -e "$CONF/crash-ignore/firefox" ]
}

@test "crash-mute keeps only the program name of a path, and refuses dots" {
  run vexos crash-mute /usr/bin/firefox
  [ -e "$CONF/crash-ignore/firefox" ]
  run vexos crash-mute ..
  [ "$status" -eq 2 ]
  run vexos crash-mute ../..
  [ "$status" -eq 2 ]
}

@test "crash-mute --json prints the status, with the mute in it" {
  run vexos crash-mute firefox --json
  [ "$status" -eq 0 ]
  jqe '.crash.muted == ["firefox"]'
  run vexos crash-mute firefox off --json
  jqe '.crash.muted == []'
  run vexos crash-mute --json
  jqe '.schema == 1'
}

# ── capture ──────────────────────────────────────────────────────────────────

@test "crash-capture with no argument prints the state" {
  run vexos crash-capture
  [ "$status" -eq 0 ]
  [ "$output" = on ]
  touch_flag() { mkdir -p "$CONF" && : >"$CONF/crash-capture-off"; }
  touch_flag
  run vexos crash-capture
  [ "$output" = off ]
}

@test "crash-capture off sets the flag the unit's ConditionPathExists=! reads, and stops the watcher" {
  run vexos crash-capture off
  [ "$status" -eq 0 ]
  [ "$output" = "Crash capture is off." ]
  [ -e "$CONF/crash-capture-off" ]
  [ "$(cat "$STUB_LOG/systemctl.log")" = "--user stop vexos-ai-crash-watch.service" ]
}

@test "crash-capture on removes the flag and starts the watcher" {
  mkdir -p "$CONF"
  : >"$CONF/crash-capture-off"
  run vexos crash-capture on
  [ "$status" -eq 0 ]
  [ "$output" = "Crash capture is on." ]
  [ ! -e "$CONF/crash-capture-off" ]
  [ "$(cat "$STUB_LOG/systemctl.log")" = "--user start vexos-ai-crash-watch.service" ]
}

@test "crash-capture saves the setting even when systemctl cannot reach a session" {
  STUB_SYSTEMCTL_RC=1 run --separate-stderr vexos crash-capture off
  [ "$status" -eq 0 ]
  [ -e "$CONF/crash-capture-off" ]
  [[ $stderr == *"vexos-ai: crash capture is off"* ]]
}

@test "crash-capture rejects anything but on and off" {
  run vexos crash-capture sometimes
  [ "$status" -eq 2 ]
  run vexos crash-capture on off
  [ "$status" -eq 2 ]
}

@test "crash-capture --json prints the new status" {
  run vexos crash-capture off --json
  [ "$status" -eq 0 ]
  jqe '.crash.capture == false'
  run vexos crash-capture on --json
  jqe '.crash.capture == true'
}

@test "crash with a pid that is not a number is a usage error" {
  run vexos crash abc
  [ "$status" -eq 2 ]
}
