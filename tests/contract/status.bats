#!/usr/bin/env bats
# `vexos-ai status --json` — schema 1 (docs/contract.md §status).

load helpers
setup() { common_setup; }

@test "a fresh machine: schema 1, defaults, main only" {
  run vexos status --json
  [ "$status" -eq 0 ]
  jqe '.schema == 1'
  jqe '.version | type == "string"'
  jqe '.installed == true'
  jqe '.needsReboot == false'
  jqe '.agent == null'
  jqe '.workdir == "/etc/nixos"'
  jqe '.activeAccount == "main"'
  jqe '.switching == {mode: "manual", threshold: 95}'
  jqe '.crash == {capture: true, muted: []}'
  jqe '.accounts | length == 1'
  jqe '.accounts[0] | .label == "main" and .active == true and .inUse == false and .signedIn == false and .plan == null and .usage == null'
}

@test "status reflects the saved settings, accounts and mutes" {
  make_account work
  set_file "$CONF/agent" opencode
  set_file "$CONF/claude-account" work
  set_file "$CONF/mode" auto
  set_file "$CONF/threshold" 80
  mkdir -p "$CONF/crash-ignore"
  touch "$CONF/crash-ignore/firefox" "$CONF/crash-ignore/steam"
  touch "$CONF/crash-capture-off"
  run vexos status --json
  [ "$status" -eq 0 ]
  jqe '.agent == "opencode" and .activeAccount == "work"'
  jqe '.switching == {mode: "auto", threshold: 80}'
  jqe '.crash == {capture: false, muted: ["firefox", "steam"]}'
  jqe '[.accounts[].label] == ["main", "work"]'
  jqe '(.accounts[] | select(.label == "work") | .active) and (.accounts[] | select(.label == "main") | .active | not)'
}

@test "a garbage agent file is null, a missing active account falls back to main" {
  set_file "$CONF/agent" emacs
  set_file "$CONF/claude-account" ghost
  run vexos status --json
  jqe '.agent == null and .activeAccount == "main"'
}

@test "signed-in state and plan come from the credentials" {
  write_creds main
  run vexos status --json
  [ "$status" -eq 0 ]
  jqe '.accounts[0].signedIn == true and .accounts[0].plan == "Max 5x"'
}

@test "usage is null for every account, even with a reading left by an older version" {
  make_account work
  write_legacy_usage main
  write_legacy_usage work
  run vexos status --json
  [ "$status" -eq 0 ]
  jqe '[.accounts[].usage] == [null, null]'
  run vexos status
  [ "$status" -eq 0 ]
  [[ $output != *"weekly"* && $output != *"usage"* && $output != *"session "[0-9]* ]]
}

@test "an expired access token is routine: still signed in; no refresh token is not" {
  write_creds main -600
  run vexos status --json
  jqe '.accounts[0].signedIn == true'
  mkdir -p "$HOME/.claude"
  jq -n '{claudeAiOauth: {accessToken: "x", expiresAt: 1000}}' >"$HOME/.claude/.credentials.json"
  run vexos status --json
  jqe '.accounts[0].signedIn == false'
}

@test "inUse is true for the account a running session uses" {
  make_account work
  fake_session 100
  fake_session 101 "$DATA/work"
  run vexos status --json
  jqe '(.accounts[] | select(.label == "main") | .inUse) and (.accounts[] | select(.label == "work") | .inUse)'
  rm -r "$VEXOS_AI_PROC/100"
  run vexos status --json
  jqe '(.accounts[] | select(.label == "main") | .inUse | not) and (.accounts[] | select(.label == "work") | .inUse)'
}

@test "needsReboot: in the running system's profile but not the booted one's" {
  mkdir -p "$VEXOS_AI_CURRENT_SYSTEM/sw/bin" "$VEXOS_AI_BOOTED_SYSTEM/sw/bin"
  run vexos status --json
  jqe '.needsReboot == false'
  touch "$VEXOS_AI_CURRENT_SYSTEM/sw/bin/vexos-ai"
  run vexos status --json
  jqe '.needsReboot == true'
  touch "$VEXOS_AI_BOOTED_SYSTEM/sw/bin/vexos-ai"
  run vexos status --json
  jqe '.needsReboot == false'
}

@test "status never touches the network or writes state, however it is asked" {
  write_creds main
  run vexos status --json
  run vexos --json status
  run vexos status
  [ "$(calls curl.count)" -eq 0 ]
  [ ! -e "$CACHE" ]
  [ ! -e "$STATE" ]
}

@test "status --json is a single JSON document on stdout, and nothing on stderr" {
  write_creds main
  run --separate-stderr vexos status --json
  [ "$status" -eq 0 ]
  [ -z "$stderr" ]
  [ "$(jq -s length <<<"$output")" -eq 1 ]
}

@test "status --refresh is accepted and ignored: same document, no network, no state" {
  write_creds main
  run vexos status --json
  local plain=$output
  run vexos status --json --refresh
  [ "$status" -eq 0 ]
  [ "$output" = "$plain" ]
  run vexos status --refresh
  [ "$status" -eq 0 ]
  [ "$(calls curl.count)" -eq 0 ]
  [ ! -e "$CACHE" ]
  [ ! -e "$STATE" ]
}

@test "status in plain text is for people and exits 0" {
  write_creds main
  set_file "$CONF/agent" claude
  run vexos status
  [ "$status" -eq 0 ]
  [[ $output == *"Assistant:  claude"* ]]
  [[ $output == *"main (Max 5x)"* ]]
}

@test "status rejects unknown arguments with exit 2" {
  run vexos status --verbose
  [ "$status" -eq 2 ]
}
