#!/usr/bin/env bats
# Launching the assistant, the `prompt` alias, global flags and exit codes.

load helpers
setup() { common_setup; }

@test "--prompt hands the agent one argv entry, on a terminal" {
  skip_without_pty
  install_agents
  set_file "$CONF/agent" claude
  run_tty --prompt "fix the thing; \$(rm -rf /)"
  [ "$status" -eq 0 ]
  grep -qx 'argc=1' "$STUB_LOG/claude.log"
  grep -qxF 'arg=fix the thing; $(rm -rf /)' "$STUB_LOG/claude.log"
}

@test "prompt <words…> is the same, words joined with spaces" {
  skip_without_pty
  install_agents
  set_file "$CONF/agent" claude
  run_tty prompt why is my wifi slow
  [ "$status" -eq 0 ]
  grep -qx 'argc=1' "$STUB_LOG/claude.log"
  grep -qx 'arg=why is my wifi slow' "$STUB_LOG/claude.log"
}

@test "flag-looking words after prompt are text, not flags" {
  skip_without_pty
  install_agents
  set_file "$CONF/agent" claude
  run_tty prompt explain --json and --non-interactive
  [ "$status" -eq 0 ]
  grep -qx 'arg=explain --json and --non-interactive' "$STUB_LOG/claude.log"
}

@test "prompt with no text is a usage error" {
  run vexos prompt
  [ "$status" -eq 2 ]
  run vexos --prompt ""
  [ "$status" -eq 2 ]
}

@test "opencode gets --prompt <text>" {
  skip_without_pty
  install_agents
  set_file "$CONF/agent" opencode
  run_tty prompt hello there
  [ "$status" -eq 0 ]
  [ "$(grep -c '^arg=' "$STUB_LOG/opencode.log")" -eq 2 ]
  grep -qx 'arg=--prompt' "$STUB_LOG/opencode.log"
  grep -qx 'arg=hello there' "$STUB_LOG/opencode.log"
}

@test "a non-main active account is used through CLAUDE_CONFIG_DIR, main is not" {
  skip_without_pty
  install_agents
  make_account work
  set_file "$CONF/agent" claude
  set_file "$CONF/claude-account" work
  run_tty
  grep -qx "config=$DATA/work" "$STUB_LOG/claude.log"
  set_file "$CONF/claude-account" main
  run_tty
  [ "$(grep -c '^config=$' "$STUB_LOG/claude.log")" -eq 1 ]
}

@test "off a terminal the assistant opens one, with the prompt as argv" {
  install_agents
  set_file "$CONF/agent" claude
  WAYLAND_DISPLAY=wayland-0 run vexos --prompt "two words"
  [ "$status" -eq 0 ]
  [ "$(cat "$STUB_LOG/terminal.log")" = "vexos-ai --prompt two words" ]
  never_called claude.log
}

@test "--non-interactive off a terminal exits 4 instead of opening one" {
  install_agents
  set_file "$CONF/agent" claude
  WAYLAND_DISPLAY=wayland-0 run vexos --non-interactive prompt hi
  [ "$status" -eq 4 ]
  nothing_opened
  WAYLAND_DISPLAY=wayland-0 VEXOS_AI_NONINTERACTIVE=1 run vexos
  [ "$status" -eq 4 ]
  nothing_opened
}

@test "no assistant chosen yet and no way to ask exits 4" {
  install_agents
  WAYLAND_DISPLAY=wayland-0 run vexos --non-interactive
  [ "$status" -eq 4 ]
  nothing_opened
}

@test "the chosen assistant not being installed is 'not found' (3)" {
  skip_without_pty
  set_file "$CONF/agent" claude
  run_tty
  [ "$status" -eq 3 ]
  set_file "$CONF/agent" opencode
  run_tty
  [ "$status" -eq 3 ]
}

@test "a corrupt agent file is a failure (1) with a pointer to pick" {
  skip_without_pty
  set_file "$CONF/agent" emacs
  run_tty
  [ "$status" -eq 1 ]
  [[ $output == *"vexos-ai pick"* ]]
}

# ── flags and codes everywhere ───────────────────────────────────────────────

@test "global flags may come before or after the command" {
  run vexos --json --non-interactive status
  [ "$status" -eq 0 ]
  jqe '.schema == 1'
  run vexos status --non-interactive --json
  jqe '.schema == 1'
}

@test "unknown commands are usage errors (2) with usage on stderr, --help is 0" {
  run --separate-stderr vexos frobnicate
  [ "$status" -eq 2 ]
  [ -z "$output" ]
  [[ $stderr == *"Usage: vexos-ai"* ]]
  run vexos --help
  [ "$status" -eq 0 ]
  [[ $output == *"Exit codes:"* ]]
}

@test "errors are one line on stderr, prefixed vexos-ai:, and nothing on stdout" {
  run --separate-stderr vexos account use ghost
  [ "$status" -eq 3 ]
  [ -z "$output" ]
  [ "$stderr" = "vexos-ai: no account named 'ghost'" ]
}

@test "the six VexPortal recipes never open a dialog or a terminal, with a TTY and a display" {
  skip_without_pty
  install_agents
  make_account work
  export WAYLAND_DISPLAY=wayland-0
  run_tty pick claude && [ "$status" -eq 0 ]
  run_tty account use work && [ "$status" -eq 0 ]
  run_tty account mode auto 90 && [ "$status" -eq 0 ]
  run_tty account mode manual "" && [ "$status" -eq 0 ]
  run_tty crash-mute firefox "" && [ "$status" -eq 0 ]
  run_tty crash-mute firefox off && [ "$status" -eq 0 ]
  run_tty crash-mute "" "" && [ "$status" -eq 0 ]
  run_tty account remove work && [ "$status" -eq 0 ]
  run_tty account add other && [ "$status" -eq 0 ]
  nothing_opened
}
