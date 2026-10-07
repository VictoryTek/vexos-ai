# shellcheck shell=bash
# Shared setup for the black-box contract tests (see docs/contract.md).
#
# Every test runs the real `vexos-ai` under a fake HOME with stub programs
# (claude, opencode, curl, systemctl, zenity, xdg-terminal-exec, notify-send,
# gsettings) first on PATH. The stubs log how they were called, so a test can
# assert that nothing opened a dialog or a terminal window.
#
# VEXOS_AI_BIN is the vexos-ai to test: the contract check passes the Nix-built
# script; by hand, `VEXOS_AI_BIN=$(nix build --print-out-paths)/bin/vexos-ai`.
# Rust Phase 2 runs this same suite against the new binary.

bats_require_minimum_version 1.5.0

TOKEN=tok-SECRET-9f3a7c1e
REFRESH=refresh-SECRET-5b2d8e4a

common_setup() {
  : "${VEXOS_AI_BIN:?set VEXOS_AI_BIN to the vexos-ai under test}"

  export HOME="$BATS_TEST_TMPDIR/home"
  mkdir -p "$HOME"
  unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_CACHE_HOME \
    WAYLAND_DISPLAY DISPLAY VEXOS_AI_NONINTERACTIVE
  CONF="$HOME/.config/vexos/ai"
  DATA="$HOME/.local/share/vexos/ai/claude-accounts"
  STATE="$HOME/.local/state/vexos/ai"
  CACHE="$HOME/.cache/vexos/ai"

  export STUB_LOG="$BATS_TEST_TMPDIR/log"
  mkdir -p "$STUB_LOG"
  local bin="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$bin"
  cp "$BATS_TEST_DIRNAME"/stubs/* "$bin"/
  chmod +x "$bin"/*
  # claude and opencode are "installed" only by tests that want them.
  mkdir -p "$BATS_TEST_TMPDIR/agents"
  mv "$bin/claude" "$bin/opencode" "$BATS_TEST_TMPDIR/agents/"
  export PATH="$bin:$PATH"

  # A fake /proc and fake system profiles; the script reads them from these.
  export VEXOS_AI_PROC="$BATS_TEST_TMPDIR/proc"
  export VEXOS_AI_CURRENT_SYSTEM="$BATS_TEST_TMPDIR/current-system"
  export VEXOS_AI_BOOTED_SYSTEM="$BATS_TEST_TMPDIR/booted-system"
  mkdir -p "$VEXOS_AI_PROC"

  write_usage_body 12.0 40.0
}

install_agents() {
  cp "$BATS_TEST_TMPDIR"/agents/* "$BATS_TEST_TMPDIR/bin/"
}

vexos() { "$VEXOS_AI_BIN" "$@"; }

# Run vexos-ai with a terminal on stdin and stdout, as in VexPortal's VTE.
have_pty() { script -qec true /dev/null >/dev/null 2>&1; }
skip_without_pty() { have_pty || skip "cannot allocate a pty here"; }
run_tty() {
  run script -qefc "$(printf '%q ' "$VEXOS_AI_BIN" "$@")" /dev/null
  output=${output//$'\r'/}
}

# Assertions ------------------------------------------------------------------

# $output is a JSON document and satisfies the jq expression.
jqe() {
  jq -e "$1" <<<"$output" >/dev/null || {
    echo "jq expression failed: $1"
    echo "$output"
    return 1
  }
}

# The stub was never called (or, given a file, the log is absent).
never_called() { [[ ! -s $STUB_LOG/$1 ]] || { echo "unexpected $1:"; cat "$STUB_LOG/$1"; return 1; }; }
calls() { if [[ -f $STUB_LOG/$1 ]]; then wc -l <"$STUB_LOG/$1"; else echo 0; fi; }

# No dialog and no terminal window: what "never prompts" means.
nothing_opened() { never_called zenity.log && never_called terminal.log; }

# State ------------------------------------------------------------------------

future() { date -u -d "+$1" +%Y-%m-%dT%H:%M:%S+00:00; }
past() { date -u -d "-$1" +%Y-%m-%dT%H:%M:%S+00:00; }

make_account() {
  local label
  for label in "$@"; do mkdir -p "$DATA/$label"; done
}

set_file() { mkdir -p "$(dirname "$1")"; printf '%s\n' "$2" >"$1"; }

# Sign-in files for an account (main by default). Expiry is seconds from now.
write_creds() { # [label] [expires-in-seconds]
  local label=${1:-main} exp=${2:-3600} dir
  if [[ $label == main ]]; then dir="$HOME/.claude"; else dir="$DATA/$label"; fi
  mkdir -p "$dir"
  jq -n --arg t "$TOKEN" --arg r "$REFRESH" --argjson e "$((($(date +%s) + exp) * 1000))" \
    '{claudeAiOauth: {accessToken: $t, refreshToken: $r, expiresAt: $e,
                      subscriptionType: "max", rateLimitTier: "default_claude_max_5x"}}' \
    >"$dir/.credentials.json"
}

# What the stub curl answers with. Percentages unless told otherwise.
write_usage_body() { # session weekly
  jq -n --argjson s "$1" --argjson w "$2" --arg sr "$(future 3hours)" --arg wr "$(future 3days)" \
    '{five_hour: {utilization: $s, resets_at: $sr},
      seven_day: {utilization: $w, resets_at: $wr},
      seven_day_opus: {utilization: ($w - 10), resets_at: $wr}}' >"$STUB_LOG/usage-body.json"
}

# A usage reading as the cache holds it (legacy keys, plus any extra jq object).
write_cache() { # label session weekly [extra-json]
  local extra=${4:-}
  [[ -n $extra ]] || extra='{}'
  mkdir -p "$CACHE"
  jq -n --argjson s "$2" --argjson w "$3" --arg sr "$(future 3hours)" --arg wr "$(future 3days)" \
    --argjson x "$extra" \
    '{ok: true, session: $s, sessionReset: $sr, weekly: $w, weeklyReset: $wr, why: ""} + $x' \
    >"$CACHE/usage-$1.json"
}

# probe state: nextAt/lastAt as seconds relative to now.
write_probe_state() { # label next-offset last-offset failures
  mkdir -p "$STATE"
  jq -n --argjson n "$(($(date +%s) + $2))" --argjson l "$(($(date +%s) + $3))" --argjson f "$4" \
    '{nextAt: $n, lastAt: $l, failures: $f}' >"$STATE/probe-$1.json"
}

probe_field() { jq -r ".$2" "$STATE/probe-$1.json"; }

# A running Claude session, in the fake /proc. Empty config means the default.
fake_session() { # pid [config-dir]
  mkdir -p "$VEXOS_AI_PROC/$1"
  printf 'claude\n' >"$VEXOS_AI_PROC/$1/comm"
  if [[ -n ${2:-} ]]; then
    printf 'CLAUDE_CONFIG_DIR=%s\0HOME=%s\0' "$2" "$HOME" >"$VEXOS_AI_PROC/$1/environ"
  else
    printf 'HOME=%s\0' "$HOME" >"$VEXOS_AI_PROC/$1/environ"
  fi
}
