#!/usr/bin/env bats
# The usage probe is gone (docs/contract.md §Credentials and the network): no
# `usage` or `usage-check` command, no request to any endpoint, and no token in
# any output or file. The curl stub is a tripwire: it fails and logs if called.

load helpers
setup() { common_setup; }

@test "usage and usage-check are unknown commands: exit 2, usage on stderr, nothing on stdout" {
  write_creds main
  set_file "$CONF/agent" claude
  for cmd in usage usage-check; do
    run --separate-stderr vexos "$cmd"
    [ "$status" -eq 2 ]
    [ -z "$output" ]
    [[ $stderr == *"Usage: vexos-ai"* ]]
    [[ $stderr != *"$TOKEN"* && $stderr != *"$REFRESH"* ]]
  done
  run --separate-stderr vexos usage main
  [ "$status" -eq 2 ]
  [ "$(calls curl.count)" -eq 0 ]
}

@test "--help does not offer a usage command" {
  run vexos --help
  [ "$status" -eq 0 ]
  [[ $output != *"print usage JSON"* ]]
  [[ $output != *"usage-check"* ]]
  [[ $output != *"their usage"* ]]
}

@test "no command calls curl, prints a token or writes one, whatever the login files hold" {
  make_account work
  write_creds main
  write_creds work
  write_legacy_usage main
  set_file "$CONF/agent" claude
  local out="" args
  for args in "status" "status --json" "status --refresh" "status --json --refresh" \
    "crash-mute" "crash-mute --json" "crash-capture" "pick claude --json"; do
    run vexos $args
    [ "$status" -eq 0 ]
    out+="$output"
  done
  [ "$(calls curl.count)" -eq 0 ]
  [[ $out != *"$TOKEN"* && $out != *"$REFRESH"* ]]
  [ -z "$(token_leaks)" ]
}

@test "the script has no usage endpoint and no network client in it" {
  run grep -nE 'api\.anthropic\.com|oauth/usage|anthropic-beta|curl -' "$VEXOS_AI_BIN"
  [ "$status" -eq 1 ]
}
