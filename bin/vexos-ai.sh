# vexos-ai — VexOS AI assistant launcher (modules/ai.nix).
#
# Opens Claude Code in a terminal, working in /etc/nixos, with the VexOS skills
# (files/ai/skills) installed in ~/.claude/skills. Also owns the pieces
# around it: first-run picker, theme sync, and "diagnose with AI" for crashes,
# failed units and failed rebuilds. Claude always runs with its default
# sign-in (~/.claude); the multiple-accounts feature is removed.
#
# Agents always start in their default, asking permission mode. Applying a
# change (nixos-rebuild switch/boot, just rebuild) is denied to them by the
# system-wide agent configs written by modules/ai.nix; the user rebuilds.
#
# The machine contract (argv, exit codes, `status --json`) is docs/contract.md.
# The short version: a command that has the argv it needs never prompts and
# never opens a dialog, even when a terminal and a display are both present.

VERSION=1.0.0
CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/vexos/ai"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/vexos/ai"
AGENT_FILE="$CONF_DIR/agent"
MUTE_DIR="$CONF_DIR/crash-ignore"
CAPTURE_OFF_FILE="$CONF_DIR/crash-capture-off"
WORKDIR=/etc/nixos
TITLE="VexOS Assistant"

# Overridable so the contract tests can run against fake system profiles.
# Nothing else reads these.
CURRENT_SYSTEM="${VEXOS_AI_CURRENT_SYSTEM:-/run/current-system}"
BOOTED_SYSTEM="${VEXOS_AI_BOOTED_SYSTEM:-/run/booted-system}"

NON_INTERACTIVE=""
case "${VEXOS_AI_NONINTERACTIVE:-}" in 1 | true | yes) NON_INTERACTIVE=1 ;; *) ;; esac
JSON=""

# ── Helpers ──────────────────────────────────────────────────────────────────

# Exit codes (docs/contract.md): 1 failure · 2 usage · 3 not found ·
# 4 needs interaction (5, busy, is reserved and no longer used).
fail() {
  local code=$1
  shift
  echo "vexos-ai: $*" >&2
  exit "$code"
}
die() { fail 1 "$@"; }
die_usage() { fail 2 "$@"; }
die_missing() { fail 3 "$@"; }
die_needs_input() { fail 4 "$@"; }

# The one human line a mutating command prints; silent under --json, where
# finish() prints the new status document instead.
say() { [[ -n $JSON ]] || printf '%s\n' "$*"; }
finish() { [[ -z $JSON ]] || status_json; }

have_gui() { [[ -n ${WAYLAND_DISPLAY:-}${DISPLAY:-} ]]; }

in_terminal() { [[ -t 0 && -t 1 ]]; }

# May a missing argument be asked for? Only with a terminal or a display, and
# never under --non-interactive.
interactive_ok() { [[ -z $NON_INTERACTIVE ]] && { in_terminal || have_gui; }; }

# Re-run this command inside a terminal window. Arguments travel as argv,
# never through a shell string, so a prompt or process name cannot be reparsed.
in_new_terminal() {
  command -v xdg-terminal-exec >/dev/null || die_missing "xdg-terminal-exec is not installed"
  exec xdg-terminal-exec vexos-ai "$@"
}

# Commands that are interactive by nature (signing in, the agent itself) need a
# terminal: use this one, or open one. --non-interactive exits 4 instead.
ensure_terminal() {
  in_terminal && return 0
  if [[ -n $NON_INTERACTIVE ]] || ! have_gui; then
    die_needs_input "this needs a terminal; run it from one (or without --non-interactive on a desktop)"
  fi
  in_new_terminal "$@"
}

read_first_line() { [[ -f $1 ]] && head -n 1 "$1"; }

default_agent() { read_first_line "$AGENT_FILE" || true; }

list_muted() {
  if [[ -d $MUTE_DIR ]]; then
    find "$MUTE_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort
  fi
}

capture_state() { if [[ -e $CAPTURE_OFF_FILE ]]; then echo off; else echo on; fi; }

# ── Picker ───────────────────────────────────────────────────────────────────

# pick_agent [claude]. With a name it only validates and saves. With none it
# asks (dialog, else terminal) and exits 4 when asking is not allowed. Claude
# Code is the only assistant; OpenCode was removed.
pick_agent() {
  local choice=${1:-}
  if [[ -n $choice ]]; then
    case "$choice" in
    claude) ;;
    *) die_usage "unknown assistant '$choice' (use claude)" ;;
    esac
  else
    interactive_ok || die_needs_input "no assistant given (use: vexos-ai pick claude)"
    if have_gui; then
      choice=$(zenity --list --radiolist --title="$TITLE" \
        --text="Choose your AI assistant." \
        --column="" --column="id" --column="Assistant" --column="About" \
        --hide-column=2 --print-column=2 --width=640 --height=260 \
        TRUE claude "Claude Code" "Anthropic. Sign in with a Claude subscription or API key.") || return 1
    else
      read -r -p "Choose assistant [claude]: " choice
    fi
    case "$choice" in
    claude) ;;
    *) echo "No assistant chosen." >&2; return 1 ;;
    esac
  fi
  mkdir -p "$CONF_DIR"
  printf '%s\n' "$choice" >"$AGENT_FILE"
}

# ── Launch ───────────────────────────────────────────────────────────────────

launch() {
  local prompt=${1:-} agent targs=()
  agent=$(default_agent)
  if [[ -z $agent ]]; then
    pick_agent || exit 1
    agent=$(default_agent)
  fi

  [[ -z $prompt ]] || targs=(--prompt "$prompt")
  ensure_terminal "${targs[@]}"

  cd "$WORKDIR" 2>/dev/null || cd "$HOME"

  case "$agent" in
  claude | opencode) # opencode: a value saved before OpenCode was removed
    command -v claude >/dev/null || die_missing "claude is not installed"
    if [[ -n $prompt ]]; then exec claude "$prompt"; else exec claude; fi
    ;;
  *) die "unknown assistant '$agent' in $AGENT_FILE (run: vexos-ai pick claude)" ;;
  esac
}

# ── Status ───────────────────────────────────────────────────────────────────

# The status document, schema 1 (docs/contract.md). Additive keys never bump
# the schema; consumers ignore keys they do not know. The account keys are
# kept with fixed values so schema 1 consumers keep parsing: the accounts
# feature is removed and Claude always uses its default sign-in.
status_json() {
  local agent reboot=false
  agent=$(default_agent)
  [[ $agent == claude ]] || agent=""
  if [[ -e $CURRENT_SYSTEM/sw/bin/vexos-ai && ! -e $BOOTED_SYSTEM/sw/bin/vexos-ai ]]; then reboot=true; fi

  # -M: jq colours its output on a terminal, and this runs in VexPortal's VTE.
  jq -ncM \
    --arg version "$VERSION" --argjson reboot "$reboot" --arg agent "$agent" \
    --arg workdir "$WORKDIR" --arg capture "$(capture_state)" \
    --argjson muted "$(list_muted | jq -Rnc '[inputs]')" '
    { schema: 1, version: $version, installed: true, needsReboot: $reboot,
      agent: (if $agent == "" then null else $agent end), workdir: $workdir,
      accounts: [], activeAccount: "main",
      switching: { mode: "manual", threshold: 95 },
      crash: { capture: ($capture == "on"), muted: $muted } }'
}

status_text() {
  status_json | jq -r '
    "Assistant:  \(.agent // "not chosen")",
    "Crashes:    capture \(if .crash.capture then "on" else "off" end)\(if (.crash.muted | length) > 0 then ", muted: \(.crash.muted | join(", "))" else "" end)",
    (if .needsReboot then "Reboot:     needed to start the background services" else empty end)'
}

status_cmd() {
  while (($#)); do
    case "$1" in
    # Accepted so existing callers keep working. There is nothing to re-read:
    # the CLI no longer takes usage readings (docs/contract.md).
    --refresh) ;;
    *) die_usage "usage: vexos-ai status [--json] [--refresh]" ;;
    esac
    shift
  done
  if [[ -n $JSON ]]; then status_json; else status_text; fi
}

# ── Theme sync ───────────────────────────────────────────────────────────────
# VexOS themes are GNOME's light/dark scheme plus the role's accent colour
# (modules/gnome-*.nix). Claude Code gets a matching custom theme, which it
# hot-reloads from ~/.claude/themes/. A theme the user picked themselves is left
# alone; only an unset theme or our own is (re)pointed at "custom:vexos".

accent_hex() {
  case "$1" in
  teal) echo "#2190a4" ;; green) echo "#3a944a" ;; yellow) echo "#c88800" ;;
  orange) echo "#ed5b00" ;; red) echo "#e62d42" ;; pink) echo "#d56199" ;;
  purple) echo "#9141ac" ;; slate) echo "#6f8396" ;; *) echo "#3584e4" ;;
  esac
}

theme_sync() {
  local scheme accent base hex home="$HOME/.claude" settings tmp
  scheme=$(gsettings get org.gnome.desktop.interface color-scheme 2>/dev/null || echo "'prefer-dark'")
  accent=$(gsettings get org.gnome.desktop.interface accent-color 2>/dev/null || echo "'blue'")
  accent=${accent//\'/}
  case "$scheme" in *prefer-light* | *default*) base=light ;; *) base=dark ;; esac
  hex=$(accent_hex "$accent")

  mkdir -p "$home/themes"
  jq -n --arg base "$base" --arg hex "$hex" '{
    name: "VexOS", base: $base,
    overrides: { claude: $hex, promptBorder: $hex, briefLabelClaude: $hex, rate_limit_fill: $hex }
  }' >"$home/themes/vexos.json.tmp" && mv "$home/themes/vexos.json.tmp" "$home/themes/vexos.json"

  settings="$home/settings.json"
  if [[ -f $settings ]]; then
    if jq -e '(.theme // "") == "" or .theme == "custom:vexos"' "$settings" >/dev/null 2>&1; then
      tmp=$(mktemp "$settings.XXXXXX")
      jq '.theme = "custom:vexos"' "$settings" >"$tmp" && mv "$tmp" "$settings"
    fi
  else
    echo '{ "theme": "custom:vexos" }' >"$settings"
  fi

  echo "Claude Code theme: $base, accent $accent ($hex)"
}

theme_watch() {
  command -v gsettings >/dev/null || exit 0
  theme_sync || true
  gsettings monitor org.gnome.desktop.interface 2>/dev/null | while read -r key _; do
    case "$key" in color-scheme: | accent-color:) theme_sync >/dev/null || true ;; esac
  done
}

# ── Diagnose ─────────────────────────────────────────────────────────────────

diagnose() {
  local unit=${1:-} facts variant
  # From the app menu: open the terminal first so collection progress shows.
  ensure_terminal diagnose ${unit:+"$unit"}
  facts=$(mktemp -t vexos-ai-diagnose.XXXXXX)
  chmod 600 "$facts"
  variant=$(read_first_line /etc/nixos/vexos-variant || echo unknown)
  echo "Collecting diagnostics…"
  [[ $unit == rebuild ]] && echo "Reproducing the rebuild failure with a dry-build (changes nothing)…"
  {
    echo "# Collected by vexos-ai diagnose at $(date -Is)"
    echo "variant: $variant"
    echo; echo "## Generations"; nixos-rebuild list-generations 2>&1 | tail -n 5
    echo; echo "## Failed system units"; systemctl --failed --no-pager 2>&1
    echo; echo "## Failed user units"; systemctl --user --failed --no-pager 2>&1
    if [[ $unit == rebuild ]]; then
      # A failed rebuild's error is printed, not journaled: reproduce it with a
      # dry-build, which evaluates everything and changes nothing.
      echo; echo "## nixos-rebuild dry-build (reproducing the failure)"
      nixos-rebuild dry-build --impure --flake "path:/etc/nixos#$variant" --show-trace 2>&1 | tail -n 150
    elif [[ -n $unit ]]; then
      echo; echo "## systemctl status $unit"; systemctl status "$unit" --no-pager 2>&1 | head -n 40
      echo; echo "## journal for $unit (this boot)"; journalctl -u "$unit" -b --no-pager 2>&1 | tail -n 200
    fi
    echo; echo "## Errors this boot"; journalctl -b -p err --no-pager 2>&1 | tail -n 200
  } >"$facts"

  launch "Something is wrong on this VexOS machine${unit:+ (about: $unit)} and I want to know why.
Facts were collected into $facts — read it first.
Use the vexos-diagnose skill (read-only diagnosis). If a fix is needed, describe it and
use the vexos skill's workflow; do not apply anything."
}

crash() {
  local pid=${1:-} comm=${2:-unknown} exe=${3:-unknown} signal=${4:-unknown} when
  [[ $pid =~ ^[0-9]+$ ]] || die_usage "usage: vexos-ai crash <pid> [comm] [exe] [signal]   (see: coredumpctl list)"
  when=$(coredumpctl list "$pid" --no-pager --no-legend 2>/dev/null | tail -n 1 | cut -d' ' -f1-4 || true)
  launch "A process crashed on this VexOS machine and I want to know why.

What systemd-coredump recorded:
  process:  $comm
  PID:      $pid
  binary:   $exe
  signal:   $signal
  time:     ${when:-unknown}

Use the vexos-diagnose skill: it covers how to investigate and what to report."
}

# crash-mute | crash-mute <prog> [on|off] | crash-mute --off <prog>
crash_mute() {
  local name=${1:-} state=${2:-on}
  if [[ $name == --off ]]; then
    name=${2:-}
    state=off
    [[ -n $name ]] || die_usage "usage: vexos-ai crash-mute --off <program>"
  fi
  if [[ -z $name ]]; then
    if [[ -n $JSON ]]; then status_json; else list_muted; fi
    return 0
  fi
  [[ $state == on || $state == off ]] || die_usage "state must be on or off"
  name=${name##*/}
  [[ -n $name && $name != . && $name != .. ]] || die_usage "not a program name"
  mkdir -p "$MUTE_DIR"
  if [[ $state == off ]]; then
    rm -f "${MUTE_DIR:?}/${name:?}"
    say "Crash notifications for '$name' are on again."
  else
    touch "$MUTE_DIR/$name"
    say "Crash notifications for '$name' are muted. Undo: vexos-ai crash-mute '$name' off"
  fi
  finish
}

# crash-capture [on|off]: whether the crash watcher runs at all. The flag file
# is read by the unit's ConditionPathExists=! (nix/module.nix) at login.
crash_capture() {
  local state=${1:-} unit=vexos-ai-crash-watch.service
  case "$state" in
  "")
    if [[ -n $JSON ]]; then status_json; else capture_state; fi
    return 0
    ;;
  on)
    rm -f "${CAPTURE_OFF_FILE:?}"
    systemctl --user start "$unit" 2>/dev/null ||
      echo "vexos-ai: crash capture is on; the watcher starts at your next login (systemctl --user start failed)" >&2
    say "Crash capture is on."
    ;;
  off)
    mkdir -p "$CONF_DIR"
    : >"$CAPTURE_OFF_FILE"
    systemctl --user stop "$unit" 2>/dev/null ||
      echo "vexos-ai: crash capture is off; the watcher stays off at login (systemctl --user stop failed)" >&2
    say "Crash capture is off."
    ;;
  *) die_usage "usage: vexos-ai crash-capture [on|off]" ;;
  esac
  finish
}

# Run by vexos-ai-crash-watch.service. Port of Omarchy's omarchy-crash-watch:
# systemd-coredump journals every dump under one MESSAGE_ID with structured
# COREDUMP_* fields (systemd.journal-fields(7)).
crash_watch() {
  local entry uid comm pid exe signal name now
  local -A last
  journalctl -f -n 0 -o json MESSAGE_ID=fc2e22bc6ee647b6b90729ab34a250b1 2>/dev/null |
    while IFS= read -r entry; do
      IFS=$'\t' read -r uid comm pid exe signal < <(
        jq -r 'def f: if . == null or . == "" then "-" else . end;
               [(._UID|f), (.COREDUMP_COMM|f), (.COREDUMP_PID|f), (.COREDUMP_EXE|f), (.COREDUMP_SIGNAL_NAME|f)] | @tsv' \
          <<<"$entry" 2>/dev/null
      ) || continue
      [[ $pid =~ ^[0-9]+$ && $uid =~ ^[0-9]+$ ]] || continue
      ((uid == UID)) || continue                      # a daemon's crash is the admin's business
      [[ -n $(default_agent) ]] || continue           # nothing to offer until an assistant is chosen
      [[ ! -e $CAPTURE_OFF_FILE ]] || continue        # capture was switched off while we ran

      name=$comm
      [[ $exe == /* ]] && name=${exe##*/}             # comm is truncated to 15 chars
      name=${name##*/}
      [[ -n $name && $name != - && $name != . && $name != .. ]] || name=unknown
      [[ $name == vexos-ai* || $name == claude ]] && continue
      [[ -e $MUTE_DIR/$name ]] && continue

      now=$(date +%s)
      (((now - ${last[$name]:-0}) < 60)) && continue  # crash loops: once a minute per program
      last[$name]=$now

      (
        action=$(notify-send -a "$TITLE" -u critical -i dialog-error \
          --action=diagnose="Diagnose with AI" --wait \
          "Process crashed: $name" "Click to have your AI assistant explain why." 2>/dev/null || true)
        [[ $action == diagnose ]] && vexos-ai crash "$pid" "$name" "$exe" "$signal"
      ) &
    done
}

# One-time invitation after the feature is enabled (vexos-ai-welcome.service).
welcome() {
  local stamp="$STATE_DIR/welcomed" action
  [[ -n $(default_agent) || -e $stamp ]] && return 0
  mkdir -p "$STATE_DIR"
  touch "$stamp"
  action=$(notify-send -a "$TITLE" -i utilities-terminal \
    --action=setup="Set up" --wait \
    "Set up your AI assistant" \
    "Let Claude Code help you change and troubleshoot VexOS." 2>/dev/null || true)
  [[ $action == setup ]] && pick_agent && in_new_terminal
}

usage() {
  cat <<'EOF'
Usage: vexos-ai [--non-interactive] [--json] [command]

  (none)                    open your AI assistant in a terminal (in /etc/nixos)
  --prompt "<text>"         open it with a task
  prompt <text…>            the same, words joined
  pick [claude]             choose the assistant (no argument: ask)
  status [--json] [--refresh]   state; --refresh is accepted and ignored
  diagnose [unit|rebuild]   collect logs (or reproduce a failed rebuild) and ask what is wrong
  crash <pid> [comm exe sig]  diagnose a core dump (see: coredumpctl list)
  crash-mute [name [on|off]]  mute / unmute / list crash notifications
  crash-mute --off <name>   unmute
  crash-capture [on|off]    watch for crashes at all (no argument: show)
  theme                     sync Claude Code's theme with the desktop

With the arguments a command needs it never asks anything. --non-interactive
(or VEXOS_AI_NONINTERACTIVE=1) forbids asking: missing input exits 4.
Exit codes: 0 ok, 1 failed, 2 bad usage, 3 not found, 4 needs interaction.
Full contract: docs/contract.md
EOF
}

# ── Dispatch ─────────────────────────────────────────────────────────────────

# Global flags may come anywhere, except inside a prompt's text. Trailing empty
# arguments count as missing: `just` passes "" for a recipe parameter left unset.
args=()
verbatim=""
for arg in "$@"; do
  if [[ -n $verbatim ]]; then args+=("$arg"); continue; fi
  case "$arg" in
  --non-interactive) NON_INTERACTIVE=1 ;;
  --json) JSON=1 ;;
  --) verbatim=1 ;;
  prompt | --prompt)
    args+=("$arg")
    [[ ${#args[@]} -ne 1 ]] || verbatim=1
    ;;
  *) args+=("$arg") ;;
  esac
done
set -- "${args[@]}"
while (($#)) && [[ -z ${!#} ]]; do set -- "${@:1:$#-1}"; done

cmd=${1:-}
[[ $# -gt 0 ]] && shift
case "$cmd" in
"") launch ;;
--prompt | prompt)
  (($#)) || die_usage "$cmd needs text"
  launch "$*"
  ;;
pick)
  [[ $# -le 1 ]] || die_usage "usage: vexos-ai pick [claude]"
  pick_agent "${1:-}" || exit 1
  say "Assistant set to Claude Code."
  finish
  ;;
status) status_cmd "$@" ;;
diagnose) diagnose "${1:-}" ;;
crash) crash "$@" ;;
crash-mute) crash_mute "$@" ;;
crash-capture)
  [[ $# -le 1 ]] || die_usage "usage: vexos-ai crash-capture [on|off]"
  crash_capture "${1:-}"
  ;;
crash-watch) crash_watch ;;
# Removed: Claude always uses its default sign-in. Callers get a clear exit 2.
account | panel)
  die_usage "'$cmd' was removed: there are no Claude accounts to manage; Claude uses its default sign-in (~/.claude)"
  ;;
theme) theme_sync ;;
theme-watch) theme_watch ;;
welcome) welcome ;;
-h | --help | help) usage ;;
*) usage >&2; exit 2 ;;
esac
