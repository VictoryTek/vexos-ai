#!/usr/bin/env bats
# `vexos-ai status --json` — schema 1 (docs/contract.md §status).

load helpers
setup() { common_setup; }

@test "a fresh machine: schema 1, defaults, no accounts" {
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
  jqe '.accounts == []'
}

@test "the top-level keys are unchanged from schema 1" {
  run vexos status --json
  jqe 'keys == (["schema", "version", "installed", "needsReboot", "agent", "workdir",
    "accounts", "activeAccount", "switching", "crash"] | sort)'
}

@test "status reflects the saved agent and mutes" {
  set_file "$CONF/agent" claude
  mkdir -p "$CONF/crash-ignore"
  touch "$CONF/crash-ignore/firefox" "$CONF/crash-ignore/steam"
  touch "$CONF/crash-capture-off"
  run vexos status --json
  [ "$status" -eq 0 ]
  jqe '.agent == "claude"'
  jqe '.crash == {capture: false, muted: ["firefox", "steam"]}'
}

@test "an agent file left by the removed OpenCode is null" {
  set_file "$CONF/agent" opencode
  run vexos status --json
  jqe '.agent == null'
}

@test "a garbage agent file is null" {
  set_file "$CONF/agent" emacs
  run vexos status --json
  jqe '.agent == null'
}

@test "what older versions left is ignored: no accounts, main, manual at 95" {
  make_account work
  set_file "$CONF/claude-account" work
  set_file "$CONF/mode" auto
  set_file "$CONF/threshold" 80
  write_creds main
  write_creds work
  write_legacy_usage main
  write_legacy_usage work
  run vexos status --json
  [ "$status" -eq 0 ]
  # signedIn and plan went with the accounts list: nothing reads the login file.
  jqe '.accounts == [] and .activeAccount == "main"'
  jqe '.switching == {mode: "manual", threshold: 95}'
  run vexos status
  [ "$status" -eq 0 ]
  [[ $output != *"work"* && $output != *"Max 5x"* && $output != *"usage"* && $output != *"weekly"* ]]
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
  [[ $output == *"Crashes:    capture on"* ]]
  [[ $output != *"Account"* && $output != *"Switching"* && $output != *"Max 5x"* ]]
}

@test "status rejects unknown arguments with exit 2" {
  run vexos status --verbose
  [ "$status" -eq 2 ]
}
