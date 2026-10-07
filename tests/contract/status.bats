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

@test "usage readings come from the cache, with plan and signed-in state" {
  write_creds main
  write_cache main 12 40 "$(jq -nc --arg r "$(future 3days)" '{fetchedAt: "2026-10-06T20:41:12+00:00", scoped: [{label: "Opus Weekly", percent: 30, resetsAt: $r}]}')"
  run vexos status --json
  [ "$status" -eq 0 ]
  jqe '.accounts[0].signedIn == true and .accounts[0].plan == "Max 5x"'
  jqe '.accounts[0].usage | .ok == true and .session == 12 and .weekly == 40 and .stale == false and .why == null'
  jqe '.accounts[0].usage | .fetchedAt == "2026-10-06T20:41:12+00:00"'
  jqe '.accounts[0].usage | (.sessionResetsAt | type == "string") and (.weeklyResetsAt | type == "string")'
  jqe '.accounts[0].usage.scoped | length == 1 and .[0].label == "Opus Weekly" and .[0].percent == 30'
}

@test "a window whose reset time has passed reads 0" {
  jq -n --arg p "$(past 1hour)" --arg f "$(future 2days)" \
    '{ok: true, session: 88, sessionReset: $p, weekly: 40, weeklyReset: $f, why: ""}' \
    >"$(mkdir -p "$CACHE" && echo "$CACHE/usage-main.json")"
  run vexos status --json
  jqe '.accounts[0].usage | .session == 0 and .weekly == 40'
}

@test "a cache written by the old script (only the original keys) still reads" {
  mkdir -p "$CACHE"
  jq -n --arg r "$(future 1hour)" '{ok: true, session: 5.5, sessionReset: $r, weekly: 6, weeklyReset: $r}' \
    >"$CACHE/usage-main.json"
  run vexos status --json
  jqe '.accounts[0].usage | .ok and .session == 5.5 and .stale == false and .why == null and (.fetchedAt | type == "string") and .scoped == []'
}

@test "a failed reading is ok:false with its reason and no numbers" {
  mkdir -p "$CACHE"
  echo '{"ok":false,"why":"not signed in"}' >"$CACHE/usage-main.json"
  run vexos status --json
  jqe '.accounts[0].usage | .ok == false and .why == "not signed in" and .session == null and .weekly == null'
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
  [ ! -e "$CACHE" ] && [ ! -e "$STATE" ]
}

@test "status --json is a single JSON document on stdout, and nothing on stderr" {
  write_creds main
  write_cache main 1 2
  run --separate-stderr vexos status --json
  [ "$status" -eq 0 ]
  [ -z "$stderr" ]
  [ "$(jq -s length <<<"$output")" -eq 1 ]
}

@test "status --refresh probes each account once, and not again within 15 seconds" {
  write_creds main
  write_usage_body 21 52
  run vexos status --json --refresh
  [ "$status" -eq 0 ]
  jqe '.accounts[0].usage | .ok and .session == 21 and .weekly == 52 and .stale == false'
  [ "$(calls curl.count)" -eq 1 ]
  run vexos status --json --refresh
  [ "$(calls curl.count)" -eq 1 ]
  jqe '.accounts[0].usage.session == 21'
}

@test "status --refresh does not override a backoff" {
  write_creds main
  write_probe_state main 600 -100 2
  run vexos status --json --refresh
  [ "$(calls curl.count)" -eq 0 ]
}

@test "status in plain text is for people and exits 0" {
  write_creds main
  write_cache main 12 40
  set_file "$CONF/agent" claude
  run vexos status
  [ "$status" -eq 0 ]
  [[ $output == *"Assistant:  claude"* ]]
  [[ $output == *"session 12%  weekly 40%"* ]]
}

@test "status rejects unknown arguments with exit 2" {
  run vexos status --verbose
  [ "$status" -eq 2 ]
}
