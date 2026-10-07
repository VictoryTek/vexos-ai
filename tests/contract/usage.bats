#!/usr/bin/env bats
# Usage probing (docs/contract.md §usage): the token never leaks, one probe at a
# time, backoff and Retry-After, last-known numbers kept as stale.

load helpers
setup() { common_setup; }

# ── the token ────────────────────────────────────────────────────────────────

@test "the token reaches curl on stdin and nowhere else" {
  write_creds main
  run vexos usage
  [ "$status" -eq 0 ]
  [ "$(calls curl.count)" -eq 1 ]
  grep -q "$TOKEN" "$STUB_LOG/curl.stdin"
  ! grep -rq -e "$TOKEN" -e "$REFRESH" "$STUB_LOG/curl.argv"
  [[ $output != *"$TOKEN"* && $output != *"$REFRESH"* ]]
  # No file the script wrote holds a token; only Claude's own credentials do.
  ! grep -rlE "$TOKEN|$REFRESH" "$HOME" | grep -v '/\.credentials\.json$'
}

@test "the token is in no output, whatever the command or the outcome" {
  write_creds main
  local out="" status_code
  for args in "usage" "status --json --refresh" "status" "account list" "usage-check"; do
    for http in 200 429 500; do
      rm -f "$STATE/probe-main.json"
      STUB_CURL_STATUS=$http run vexos $args
      out+="$output"
    done
  done
  STUB_CURL_FAIL=7 run vexos usage
  out+="$output"
  [[ $out != *"$TOKEN"* && $out != *"$REFRESH"* ]]
  ! grep -rlE "$TOKEN|$REFRESH" "$HOME" | grep -v '/\.credentials\.json$'
}

# ── the reading ──────────────────────────────────────────────────────────────

@test "a good reading keeps the original keys and adds the new ones" {
  write_creds main
  run vexos usage
  [ "$status" -eq 0 ]
  jqe '.ok == true and .session == 12 and .weekly == 40 and .why == ""'
  jqe '(.sessionReset | type == "string") and (.weeklyReset | type == "string")'
  jqe '.stale == false and .plan == "Max 5x" and (.fetchedAt | type == "string")'
  jqe '.scoped | length == 1 and .[0].label == "Opus Weekly" and .[0].percent == 30'
  diff <(jq -S . <<<"$output") <(jq -S . "$CACHE/usage-main.json")
}

@test "the cache keeps types released VexPortal can parse: ok bool, numbers, why string" {
  write_creds main
  vexos usage >/dev/null
  jq -e '(.ok | type) == "boolean" and (.session | type) == "number" and (.weekly | type) == "number" and (.why | type) == "string"' "$CACHE/usage-main.json"
  STUB_CURL_STATUS=500 run vexos status --refresh
  rm -f "$STATE/probe-main.json"
  STUB_CURL_STATUS=500 vexos usage >/dev/null
  jq -e '(.ok | type) == "boolean" and (.session | type) == "number" and (.weekly | type) == "number" and (.why | type) == "string"' "$CACHE/usage-main.json"
}

@test "utilisation as a 0-1 fraction is read as a percentage" {
  write_creds main
  write_usage_body 0.25 0.5
  run vexos usage
  jqe '.session == 25 and .weekly == 50'
}

@test "usage of an unknown account exits 3" {
  run vexos usage ghost
  [ "$status" -eq 3 ]
}

# ── when it probes ───────────────────────────────────────────────────────────

@test "the active account is probed at most every 10 minutes" {
  write_creds main
  write_cache main 5 5
  write_probe_state main 0 -60 0
  run vexos usage-check
  [ "$(calls curl.count)" -eq 0 ]
  write_probe_state main 0 -700 0
  set_file "$CONF/agent" claude
  run vexos usage-check
  [ "$(calls curl.count)" -eq 1 ]
}

@test "within 15 points of the threshold it is probed every 3 minutes" {
  write_creds main
  set_file "$CONF/agent" claude
  write_cache main 85 10 # threshold 95
  write_probe_state main 0 -200 0
  run vexos usage-check
  [ "$(calls curl.count)" -eq 1 ]
}

@test "inactive accounts are probed every 60 minutes" {
  make_account work
  write_creds work
  set_file "$CONF/agent" claude
  write_probe_state work 0 -1800 0
  write_probe_state main 0 -30 0
  run vexos usage-check
  [ "$(calls curl.count)" -eq 0 ]
  write_probe_state work 0 -3700 0
  run vexos usage-check
  [ "$(calls curl.count)" -eq 1 ]
}

@test "only one probe per account runs at a time (flock)" {
  write_creds main
  set_file "$CONF/agent" claude
  mkdir -p "$STATE"
  # One process that holds the lock until it is killed (no child to outlive it).
  bash -c 'exec 9>"$1"; flock -n 9 && exec sleep 30' _ "$STATE/probe-main.lock" &
  local holder=$!
  sleep 0.3
  run vexos usage-check
  [ "$(calls curl.count)" -eq 0 ]
  kill "$holder" 2>/dev/null || true
  wait "$holder" 2>/dev/null || true
  run vexos usage-check
  [ "$(calls curl.count)" -eq 1 ]
}

# ── failing ──────────────────────────────────────────────────────────────────

@test "429 honours Retry-After, keeps the last numbers as stale, and then stays quiet" {
  write_creds main
  vexos usage >/dev/null
  write_probe_state main 0 -100 0
  STUB_CURL_STATUS=429 STUB_CURL_RETRY_AFTER=120 run vexos usage
  [ "$status" -eq 0 ]
  jqe '.ok == true and .session == 12 and .stale == true and (.why | test("rate limited"))'
  local next now
  next=$(probe_field main nextAt)
  now=$(date +%s)
  ((next >= now + 115 && next <= now + 125))
  [ "$(probe_field main failures)" -eq 1 ]
  # Backing off: neither the timer nor --refresh goes to the network.
  local before
  before=$(calls curl.count)
  run vexos usage-check
  run vexos status --refresh
  [ "$(calls curl.count)" -eq "$before" ]
}

@test "429 without Retry-After backs off 1, 2, 4 … 60 minutes" {
  write_creds main
  local expect=(60 120 240 480 960 1920 3600 3600) i
  for i in 0 1 2 3 4 5 6 7; do
    write_probe_state main -1 -100 "$i"
    STUB_CURL_STATUS=429 vexos usage >/dev/null
    local delay
    delay=$(($(probe_field main nextAt) - $(date +%s)))
    ((delay >= ${expect[$i]} - 5 && delay <= ${expect[$i]} + 1))
    [ "$(probe_field main failures)" -eq $((i + 1)) ]
  done
}

@test "a server error backs off the same way" {
  write_creds main
  STUB_CURL_STATUS=503 run vexos usage
  jqe '.ok == false and (.why | test("unavailable"))'
  [ "$(probe_field main failures)" -eq 1 ]
  [ "$(($(probe_field main nextAt) - $(date +%s)))" -ge 55 ]
}

@test "a transport error retries in a minute and does not count as a failure" {
  write_creds main
  STUB_CURL_FAIL=7 run vexos usage
  [ "$status" -eq 0 ]
  jqe '.ok == false and (.why | test("cannot reach"))'
  [ "$(probe_field main failures)" -eq 0 ]
  local delay
  delay=$(($(probe_field main nextAt) - $(date +%s)))
  ((delay >= 55 && delay <= 61))
}

@test "success clears a backoff" {
  write_creds main
  write_probe_state main -1 -100 3
  run vexos usage
  jqe '.ok == true'
  [ "$(probe_field main failures)" -eq 0 ]
  [ "$(probe_field main nextAt)" -eq 0 ]
}

@test "an unknown payload is 'usage format changed', not a crash, and keeps the numbers" {
  write_creds main
  vexos usage >/dev/null
  write_probe_state main 0 -100 0
  echo '{"quota":{"left":3}}' >"$BATS_TEST_TMPDIR/odd.json"
  STUB_CURL_BODY="$BATS_TEST_TMPDIR/odd.json" run vexos usage
  [ "$status" -eq 0 ]
  jqe '.ok == true and .stale == true and .why == "usage format changed" and .session == 12'
  echo 'not json at all <html>' >"$BATS_TEST_TMPDIR/odd.json"
  write_probe_state main 0 -100 0
  STUB_CURL_BODY="$BATS_TEST_TMPDIR/odd.json" run vexos usage
  [ "$status" -eq 0 ]
  jqe '.why == "usage format changed"'
}

@test "with no earlier numbers a failure is ok:false and says why" {
  write_creds main
  STUB_CURL_STATUS=401 run vexos usage
  jqe '.ok == false and .stale == false and (.why | test("rejected"))'
}

@test "an expired access token is not sent, says to start Claude Code, and is not a backoff" {
  write_creds main -600
  run vexos usage
  [ "$status" -eq 0 ]
  [ "$(calls curl.count)" -eq 0 ]
  jqe '.ok == false and .why == "start Claude Code to refresh it"'
  [ ! -e "$STATE/probe-main.json" ]
  run vexos status --json
  jqe '.accounts[0].signedIn == true'
}

@test "no credentials at all: not signed in, no request" {
  run vexos usage
  jqe '.ok == false and .why == "not signed in"'
  [ "$(calls curl.count)" -eq 0 ]
  run vexos status --json
  jqe '.accounts[0].signedIn == false'
}

@test "the earlier numbers survive an expired token, marked stale" {
  write_creds main
  vexos usage >/dev/null
  write_creds main -600
  write_probe_state main 0 -700 0
  run vexos usage-check
  set_file "$CONF/agent" claude
  run vexos usage-check
  run vexos status --json
  jqe '.accounts[0].usage | .ok and .session == 12 and .stale and .why == "start Claude Code to refresh it"'
}

# ── warnings (existing behaviour, kept) ──────────────────────────────────────

@test "usage-check warns once per limit window near the threshold" {
  write_creds main
  set_file "$CONF/agent" claude
  write_usage_body 97 20
  run vexos usage-check
  [ "$status" -eq 0 ]
  grep -q "is at 97%" "$STUB_LOG/notify.log"
  [ "$(calls notify.log)" -eq 1 ]
  write_probe_state main 0 -700 0
  run vexos usage-check
  [ "$(calls notify.log)" -eq 1 ]
}

@test "usage-check in auto mode moves new sessions to the account with most headroom" {
  make_account work
  write_creds main
  write_creds work
  set_file "$CONF/agent" claude
  set_file "$CONF/mode" auto
  write_usage_body 97 20
  write_cache work 10 15
  write_probe_state work 3600 0 0
  run vexos usage-check
  [ "$status" -eq 0 ]
  [ "$(cat "$CONF/claude-account")" = work ]
}

@test "usage-check does nothing until Claude is the chosen assistant" {
  write_creds main
  set_file "$CONF/agent" opencode
  run vexos usage-check
  [ "$(calls curl.count)" -eq 0 ]
}
