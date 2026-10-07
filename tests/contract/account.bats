#!/usr/bin/env bats
# `vexos-ai account …` — ai-account-add / use / remove / mode, and rename.

load helpers
setup() { common_setup; }

# ── use ──────────────────────────────────────────────────────────────────────

@test "account use <label> saves it, without asking anything" {
  make_account work
  WAYLAND_DISPLAY=wayland-0 run vexos account use work
  [ "$status" -eq 0 ]
  [ "$output" = "New Claude sessions now use 'work'. Running sessions keep their account." ]
  [ "$(cat "$CONF/claude-account")" = work ]
  nothing_opened
}

@test "account use main works" {
  make_account work
  set_file "$CONF/claude-account" work
  run vexos account use main
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/claude-account")" = main ]
}

@test "account use next walks main, the labels in order, then main again" {
  make_account alpha beta
  run vexos account use next
  [ "$(cat "$CONF/claude-account")" = alpha ]
  run vexos account use next
  [ "$(cat "$CONF/claude-account")" = beta ]
  run vexos account use next
  [ "$(cat "$CONF/claude-account")" = main ]
}

@test "account use with an empty target means next" {
  make_account alpha
  run vexos account use ""
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/claude-account")" = alpha ]
}

@test "account use of an account that does not exist exits 3" {
  run vexos account use ghost
  [ "$status" -eq 3 ]
  [[ $output == "vexos-ai: no account named 'ghost'" ]]
  [ ! -e "$CONF/claude-account" ]
}

@test "account use refuses a path, not a label (exit 2)" {
  mkdir -p "$HOME/.local/share/vexos/ai/escape"
  run vexos account use ../escape
  [ "$status" -eq 2 ]
  [ ! -e "$CONF/claude-account" ]
}

@test "account use --json prints the new status" {
  make_account work
  run vexos account use work --json
  [ "$status" -eq 0 ]
  jqe '.activeAccount == "work" and (.accounts[] | select(.label == "work") | .active)'
}

# ── remove ───────────────────────────────────────────────────────────────────

@test "account remove deletes the account and what is kept about it" {
  make_account work
  set_file "$CONF/claude-account" work
  write_cache work 10 20
  write_probe_state work 0 0 0
  run vexos account remove work
  [ "$status" -eq 0 ]
  [ "$output" = "Removed 'work'." ]
  [ ! -e "$DATA/work" ]
  [ ! -e "$CONF/claude-account" ]
  [ ! -e "$CACHE/usage-work.json" ]
  [ ! -e "$STATE/probe-work.json" ]
}

@test "account remove leaves the active account alone when another is removed" {
  make_account work play
  set_file "$CONF/claude-account" play
  run vexos account remove work
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/claude-account")" = play ]
}

@test "account remove of an unknown account exits 3" {
  run vexos account remove ghost
  [ "$status" -eq 3 ]
}

@test "account remove main is a usage error" {
  run vexos account remove main
  [ "$status" -eq 2 ]
}

@test "account remove with no label, or a bad one, is a usage error" {
  run vexos account remove
  [ "$status" -eq 2 ]
  run vexos account remove 'a/b'
  [ "$status" -eq 2 ]
}

@test "account remove while a session uses the account exits 5 and keeps it" {
  make_account work
  fake_session 4242 "$DATA/work"
  run vexos account remove work
  [ "$status" -eq 5 ]
  [ -d "$DATA/work" ]
}

@test "account remove is fine when the running session uses another account" {
  make_account work play
  fake_session 4242 "$DATA/play"
  run vexos account remove work
  [ "$status" -eq 0 ]
}

# ── rename ───────────────────────────────────────────────────────────────────

@test "account rename moves the account, the active pointer and the cache" {
  make_account work
  set_file "$CONF/claude-account" work
  write_cache work 10 20
  write_probe_state work 0 0 0
  touch "$DATA/work/marker"
  run vexos account rename work office
  [ "$status" -eq 0 ]
  [ "$output" = "Renamed 'work' to 'office'." ]
  [ -e "$DATA/office/marker" ]
  [ ! -e "$DATA/work" ]
  [ "$(cat "$CONF/claude-account")" = office ]
  [ -e "$CACHE/usage-office.json" ]
  [ -e "$STATE/probe-office.json" ]
  [ ! -e "$CACHE/usage-work.json" ]
}

@test "account rename keeps another active account as it was" {
  make_account work play
  set_file "$CONF/claude-account" play
  run vexos account rename work office
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/claude-account")" = play ]
}

@test "account rename: unknown 3, taken 2, main 2, reserved or malformed 2, missing 2" {
  make_account work play
  run vexos account rename ghost office
  [ "$status" -eq 3 ]
  run vexos account rename work play
  [ "$status" -eq 2 ]
  run vexos account rename main office
  [ "$status" -eq 2 ]
  run vexos account rename work main
  [ "$status" -eq 2 ]
  run vexos account rename work next
  [ "$status" -eq 2 ]
  run vexos account rename work 'a b'
  [ "$status" -eq 2 ]
  run vexos account rename work
  [ "$status" -eq 2 ]
  [ -d "$DATA/work" ]
}

@test "account rename while a session uses the account exits 5" {
  make_account work
  fake_session 4242 "$DATA/work"
  run vexos account rename work office
  [ "$status" -eq 5 ]
  [ -d "$DATA/work" ]
}

# ── mode ─────────────────────────────────────────────────────────────────────

@test "account mode <mode> <pct> saves both" {
  run vexos account mode auto 90
  [ "$status" -eq 0 ]
  [ "$output" = "Near 90% of a limit, new sessions switch to the account with the most headroom." ]
  [ "$(cat "$CONF/mode")" = auto ]
  [ "$(cat "$CONF/threshold")" = 90 ]
  run vexos account mode manual
  [ "$status" -eq 0 ]
  [ "$output" = "Near 90% of a limit you get a notification; switching is up to you." ]
  [ "$(cat "$CONF/mode")" = manual ]
  [ "$(cat "$CONF/threshold")" = 90 ]
}

@test "account mode treats an empty pct as missing, as just passes it" {
  WAYLAND_DISPLAY=wayland-0 run vexos account mode auto ""
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/mode")" = auto ]
  [ ! -e "$CONF/threshold" ]
  nothing_opened
}

@test "account mode rejects a bad mode or threshold (exit 2) and writes nothing" {
  run vexos account mode sometimes
  [ "$status" -eq 2 ]
  run vexos account mode auto 0
  [ "$status" -eq 2 ]
  run vexos account mode auto 101
  [ "$status" -eq 2 ]
  run vexos account mode auto 95x
  [ "$status" -eq 2 ]
  [ ! -e "$CONF/mode" ]
  [ ! -e "$CONF/threshold" ]
}

@test "account mode reads 089 as 89, not as a broken octal number" {
  run vexos account mode auto 089
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/threshold")" = 89 ]
}

@test "account mode with no arguments just reports" {
  run vexos account mode
  [ "$status" -eq 0 ]
  [ "$output" = "Near 95% of a limit you get a notification; switching is up to you." ]
  [ ! -e "$CONF/mode" ]
}

@test "account mode --json prints the new status" {
  run vexos --json account mode auto 80
  [ "$status" -eq 0 ]
  jqe '.switching == {mode: "auto", threshold: 80}'
}

# ── add ──────────────────────────────────────────────────────────────────────

@test "account add on a terminal signs in with the new account's own config dir" {
  skip_without_pty
  install_agents
  WAYLAND_DISPLAY=wayland-0 run_tty account add work
  [ "$status" -eq 0 ]
  [[ $output == *"Added 'work'"* ]]
  [ -e "$DATA/work/.credentials.json" ]
  grep -qx "arg=login" "$STUB_LOG/claude.log"
  grep -qx "config=$DATA/work" "$STUB_LOG/claude.log"
  [ "$(stat -c %a "$DATA/work")" = 700 ]
  nothing_opened
}

@test "account add --json keeps stdout for the status document" {
  skip_without_pty
  install_agents
  run_tty account add work --json
  [ "$status" -eq 0 ]
  # The sign-in chatter goes to stderr, which script merges into the output:
  # find the document rather than assume it is the only thing there.
  grep '^{' <<<"$output" | jq -e '.accounts | map(.label) | index("work") != null' >/dev/null
}

@test "account add without a terminal opens one, with the label as argv" {
  install_agents
  WAYLAND_DISPLAY=wayland-0 run vexos account add work
  [ "$status" -eq 0 ]
  [ "$(cat "$STUB_LOG/terminal.log")" = "vexos-ai account add work" ]
  never_called zenity.log
  [ ! -e "$DATA/work" ]
}

@test "account add --non-interactive without a terminal exits 4 and opens nothing" {
  install_agents
  WAYLAND_DISPLAY=wayland-0 run vexos account add work --non-interactive
  [ "$status" -eq 4 ]
  nothing_opened
  [ ! -e "$DATA/work" ]
}

@test "account add with no terminal and no display exits 4" {
  install_agents
  run vexos account add work
  [ "$status" -eq 4 ]
}

@test "account add with no label and --non-interactive exits 4" {
  install_agents
  WAYLAND_DISPLAY=wayland-0 run vexos account add --non-interactive
  [ "$status" -eq 4 ]
  nothing_opened
}

@test "account add rejects a bad or reserved label (exit 2)" {
  install_agents
  for label in main next 'a b' 'a/b' '' ; do
    [ -z "$label" ] && continue
    run vexos account add "$label"
    [ "$status" -eq 2 ]
  done
  run vexos account add "$(printf 'x%.0s' {1..33})"
  [ "$status" -eq 2 ]
}

@test "account add of an existing label exits 2" {
  install_agents
  make_account work
  run vexos account add work
  [ "$status" -eq 2 ]
}

@test "account add without claude installed exits 3" {
  run vexos account add work
  [ "$status" -eq 3 ]
}

@test "account add cleans up when the sign-in does not finish" {
  skip_without_pty
  install_agents
  STUB_CLAUDE_LOGIN_FAIL=1 run_tty account add work
  [ "$status" -eq 1 ]
  [ ! -e "$DATA/work" ]
}

# ── list ─────────────────────────────────────────────────────────────────────

@test "account list marks the active account and shows readings" {
  make_account work
  set_file "$CONF/claude-account" work
  write_cache work 31 62
  write_probe_state work 3600 0 0 # backing off: listing must not probe
  run vexos account list
  [ "$status" -eq 0 ]
  [[ ${lines[0]} == "  main"*"usage unknown" ]]
  [[ ${lines[1]} == "* work"*"session 31%  weekly 62%" ]]
}

@test "account (no subcommand) lists, an unknown one is a usage error" {
  run vexos account
  [ "$status" -eq 0 ]
  run vexos account frobnicate
  [ "$status" -eq 2 ]
}
