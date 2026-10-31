#!/usr/bin/env bats
# `vexos-ai pick` — the ai-pick recipe. With the argument it never asks.

load helpers
setup() { common_setup; }

@test "pick <agent> saves the choice, prints one line, exits 0" {
  run vexos pick claude
  [ "$status" -eq 0 ]
  [ "$output" = "Assistant set to Claude Code." ]
  [ "$(cat "$CONF/agent")" = claude ]
}

@test "pick <agent> with a terminal AND a display open nothing (VexPortal's VTE)" {
  skip_without_pty
  WAYLAND_DISPLAY=wayland-0 run_tty pick claude
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/agent")" = claude ]
  nothing_opened
}

@test "pick <agent> with only a display opens nothing" {
  WAYLAND_DISPLAY=wayland-0 run vexos pick claude
  [ "$status" -eq 0 ]
  nothing_opened
}

@test "pick takes an empty argument as missing, as just passes it" {
  WAYLAND_DISPLAY=wayland-0 VEXOS_AI_NONINTERACTIVE=1 run vexos pick ""
  [ "$status" -eq 4 ]
  nothing_opened
}

@test "pick with an unknown agent is a usage error and changes nothing" {
  set_file "$CONF/agent" claude
  run vexos pick gemini
  [ "$status" -eq 2 ]
  [[ $output == "vexos-ai: unknown assistant 'gemini'"* ]]
  [ "$(cat "$CONF/agent")" = claude ]
}

@test "pick opencode is a usage error: it was removed" {
  set_file "$CONF/agent" claude
  run --separate-stderr vexos pick opencode
  [ "$status" -eq 2 ]
  [ -z "$output" ]
  [ "$stderr" = "vexos-ai: unknown assistant 'opencode' (use claude)" ]
  [ "$(cat "$CONF/agent")" = claude ]
}

@test "pick with too many arguments is a usage error" {
  run vexos pick claude claude
  [ "$status" -eq 2 ]
}

@test "pick with no argument and no terminal or display exits 4" {
  run vexos pick
  [ "$status" -eq 4 ]
  [ ! -e "$CONF/agent" ]
  nothing_opened
}

@test "pick with no argument and --non-interactive exits 4 even with a display" {
  WAYLAND_DISPLAY=wayland-0 run vexos pick --non-interactive
  [ "$status" -eq 4 ]
  nothing_opened
}

@test "pick with no argument honours VEXOS_AI_NONINTERACTIVE=1 on a terminal too" {
  skip_without_pty
  WAYLAND_DISPLAY=wayland-0 VEXOS_AI_NONINTERACTIVE=1 run_tty pick
  [ "$status" -eq 4 ]
  nothing_opened
}

@test "pick with no argument falls back to the dialog when there is a display" {
  WAYLAND_DISPLAY=wayland-0 STUB_ZENITY_ANSWER=claude run vexos pick
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/agent")" = claude ]
  [ "$(calls zenity.log)" -eq 1 ]
}

@test "pick --json prints the new status document and nothing else" {
  run vexos pick claude --json
  [ "$status" -eq 0 ]
  jqe '.schema == 1 and .agent == "claude"'
  [ "$(jq -s length <<<"$output")" -eq 1 ]
}
