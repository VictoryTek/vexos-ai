# vexos-ai — VexOS AI assistant launcher (modules/ai.nix).
#
# Opens the user's chosen coding agent (Claude Code or OpenCode) in a terminal,
# working in /etc/nixos, with the VexOS skills (files/ai/skills) installed in
# ~/.claude/skills — both agents read that directory. Also owns the pieces
# around it: first-run picker, multiple Claude accounts, theme sync, and
# "diagnose with AI" for crashes, failed units and failed rebuilds.
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
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/vexos/ai"
ACCOUNTS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/vexos/ai/claude-accounts"
AGENT_FILE="$CONF_DIR/agent"
ACCOUNT_FILE="$CONF_DIR/claude-account"
MODE_FILE="$CONF_DIR/mode"
THRESHOLD_FILE="$CONF_DIR/threshold"
MUTE_DIR="$CONF_DIR/crash-ignore"
CAPTURE_OFF_FILE="$CONF_DIR/crash-capture-off"
WORKDIR=/etc/nixos
TITLE="VexOS Assistant"

# Overridable so the contract tests can run against a fake /proc and fake
# system profiles. Nothing else reads these.
PROC_DIR="${VEXOS_AI_PROC:-/proc}"
CURRENT_SYSTEM="${VEXOS_AI_CURRENT_SYSTEM:-/run/current-system}"
BOOTED_SYSTEM="${VEXOS_AI_BOOTED_SYSTEM:-/run/booted-system}"

NON_INTERACTIVE=""
case "${VEXOS_AI_NONINTERACTIVE:-}" in 1 | true | yes) NON_INTERACTIVE=1 ;; *) ;; esac
JSON=""

# ── Helpers ──────────────────────────────────────────────────────────────────

# Exit codes (docs/contract.md): 1 failure · 2 usage · 3 not found ·
# 4 needs interaction · 5 busy.
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
die_busy() { fail 5 "$@"; }

# The one human line a mutating command prints; silent under --json, where
# finish() prints the new status document instead.
say() { [[ -n $JSON ]] || printf '%s\n' "$*"; }
finish() { [[ -z $JSON ]] || status_json; }

# Where a conversation with the user goes: stdout, unless that is reserved for
# the --json document.
human_fd() { if [[ -n $JSON ]]; then echo 2; else echo 1; fi; }

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

# A free-text answer, from the terminal or a dialog.
ask_text() {
  local answer=""
  if in_terminal; then
    read -r -p "$1 " answer
  else
    answer=$(zenity --entry --title="$TITLE" --text="$1") || return 1
  fi
  printf '%s\n' "$answer"
}

now_s() { date +%s; }

read_first_line() { [[ -f $1 ]] && head -n 1 "$1"; }

default_agent() { read_first_line "$AGENT_FILE" || true; }

active_account() {
  local a
  a=$(read_first_line "$ACCOUNT_FILE" || true)
  if [[ -n $a && $a != main && -d $ACCOUNTS_DIR/$a ]]; then echo "$a"; else echo main; fi
}

# Claude's own default home stays implicit for the main account: Claude ties
# ~/.claude.json to it, so CLAUDE_CONFIG_DIR is only ever set for the others.
account_home() {
  if [[ $1 == main ]]; then echo "$HOME/.claude"; else echo "$ACCOUNTS_DIR/$1"; fi
}

list_accounts() {
  echo main
  if [[ -d $ACCOUNTS_DIR ]]; then
    find "$ACCOUNTS_DIR" -mindepth 1 -maxdepth 1 -type d ! -name main -printf '%f\n' | sort
  fi
}

valid_label() { [[ $1 =~ ^[A-Za-z0-9_-]{1,32}$ && $1 != main ]]; }
# "next" is what `account use` takes to mean the following account.
valid_new_label() { valid_label "$1" && [[ $1 != next ]]; }

# An account that must already exist: main, or a label with a directory.
require_account() {
  [[ $1 == main ]] && return 0
  valid_label "$1" || die_usage "not a valid account label: '$1'"
  [[ -d $ACCOUNTS_DIR/$1 ]] || die_missing "no account named '$1'"
}

# A whole percentage 1-100. The 10# keeps "089" from being read as octal.
valid_percent() { [[ $1 =~ ^[0-9]{1,3}$ ]] && ((10#$1 >= 1 && 10#$1 <= 100)); }

threshold() {
  local t
  t=$(read_first_line "$THRESHOLD_FILE" || true)
  if valid_percent "$t"; then echo $((10#$t)); else echo 95; fi
}

switch_mode() {
  if [[ $(read_first_line "$MODE_FILE" || true) == auto ]]; then echo auto; else echo manual; fi
}

list_muted() {
  if [[ -d $MUTE_DIR ]]; then
    find "$MUTE_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort
  fi
}

capture_state() { if [[ -e $CAPTURE_OFF_FILE ]]; then echo off; else echo on; fi; }

# Labels of the accounts a running Claude Code session is using, one per line.
# A session's account is its CLAUDE_CONFIG_DIR; none means main. Only our own
# processes are readable, which is the point: they are the user's sessions.
sessions_in_use() {
  local p comm dir
  for p in "$PROC_DIR"/[0-9]*; do
    comm=$(head -n 1 "$p/comm" 2>/dev/null) || continue
    case "$comm" in
    claude | .claude-wrapped) ;;
    node | bun) tr '\0' ' ' <"$p/cmdline" 2>/dev/null | grep -q claude || continue ;;
    *) continue ;;
    esac
    dir=$(tr '\0' '\n' <"$p/environ" 2>/dev/null | grep -a '^CLAUDE_CONFIG_DIR=' | head -n 1 | cut -d= -f2- || true)
    if [[ -z $dir || $dir == "$HOME/.claude" ]]; then
      echo main
    elif [[ $dir == "$ACCOUNTS_DIR"/* ]]; then
      echo "${dir#"$ACCOUNTS_DIR"/}"
    fi
  done
}

account_in_use() { sessions_in_use | grep -qx "$1"; }

# ── Picker ───────────────────────────────────────────────────────────────────

# pick_agent [claude|opencode]. With a name it only validates and saves. With
# none it asks (dialog, else terminal) and exits 4 when asking is not allowed.
pick_agent() {
  local choice=${1:-}
  if [[ -n $choice ]]; then
    case "$choice" in
    claude | opencode) ;;
    *) die_usage "unknown assistant '$choice' (use claude or opencode)" ;;
    esac
  else
    interactive_ok || die_needs_input "no assistant given (use: vexos-ai pick claude|opencode)"
    if have_gui; then
      choice=$(zenity --list --radiolist --title="$TITLE" \
        --text="Choose your AI assistant. You can change this later from the VexOS Assistant menu." \
        --column="" --column="id" --column="Assistant" --column="About" \
        --hide-column=2 --print-column=2 --width=640 --height=260 \
        TRUE claude "Claude Code" "Anthropic. Sign in with a Claude subscription or API key." \
        FALSE opencode "OpenCode" "Open source. Claude, ChatGPT, local models and more.") || return 1
    else
      read -r -p "Choose assistant [claude/opencode]: " choice
    fi
    case "$choice" in
    claude | opencode) ;;
    *) echo "No assistant chosen." >&2; return 1 ;;
    esac
  fi
  mkdir -p "$CONF_DIR"
  printf '%s\n' "$choice" >"$AGENT_FILE"
}

agent_title() { if [[ $1 == claude ]]; then echo "Claude Code"; else echo "OpenCode"; fi; }

# ── Launch ───────────────────────────────────────────────────────────────────

launch() {
  local prompt=${1:-} agent acct targs=()
  agent=$(default_agent)
  if [[ -z $agent ]]; then
    pick_agent || exit 1
    agent=$(default_agent)
  fi

  [[ -z $prompt ]] || targs=(--prompt "$prompt")
  ensure_terminal "${targs[@]}"

  cd "$WORKDIR" 2>/dev/null || cd "$HOME"

  case "$agent" in
  claude)
    command -v claude >/dev/null || die_missing "claude is not installed"
    acct=$(active_account)
    if [[ $acct != main ]]; then
      CLAUDE_CONFIG_DIR=$(account_home "$acct")
      export CLAUDE_CONFIG_DIR
    fi
    if [[ -n $prompt ]]; then exec claude "$prompt"; else exec claude; fi
    ;;
  opencode)
    command -v opencode >/dev/null || die_missing "opencode is not installed"
    if [[ -n $prompt ]]; then exec opencode --prompt "$prompt"; else exec opencode; fi
    ;;
  *) die "unknown assistant '$agent' in $AGENT_FILE (run: vexos-ai pick claude|opencode)" ;;
  esac
}

# ── Accounts (Claude) ────────────────────────────────────────────────────────

account_add() {
  local label=${1:-} home fd
  if [[ -z $label ]]; then
    interactive_ok || die_needs_input "no account name given (use: vexos-ai account add <label>)"
    label=$(ask_text "Name for the new Claude account (e.g. work):") || die_needs_input "no account name given"
  fi
  valid_new_label "$label" || die_usage "account label must be 1-32 letters, digits, - or _ (and not 'main' or 'next')"
  home=$(account_home "$label")
  [[ ! -e $home ]] || die_usage "account '$label' already exists"
  command -v claude >/dev/null || die_missing "claude is not installed"
  ensure_terminal account add "$label"

  mkdir -p "$home"
  chmod 700 "$home"
  mkdir -p "$HOME/.claude/skills"
  ln -sfn "$HOME/.claude/skills" "$home/skills"
  theme_sync >/dev/null 2>&1 || true

  # The browser sign-in is a conversation with the user: under --json stdout is
  # reserved for the status document, so it goes to stderr.
  fd=$(human_fd)
  {
    echo "Signing in a new Claude account: $label"
    echo "Tip: if your browser is already signed in to another Claude account,"
    echo "open the sign-in link in a private window."
    echo
  } >&"$fd"
  if CLAUDE_CONFIG_DIR="$home" claude auth login >&"$fd"; then
    say ""
    say "Added '$label'. Switch to it with: vexos-ai account use $label"
    finish
  else
    rm -rf "${home:?}"
    die "sign-in did not finish; nothing was added"
  fi
}

account_use() {
  local target=${1:-next} current accounts i
  current=$(active_account)
  mapfile -t accounts < <(list_accounts)
  if [[ $target == next ]]; then
    target=${accounts[0]}
    for i in "${!accounts[@]}"; do
      if [[ ${accounts[$i]} == "$current" ]]; then
        target=${accounts[$(((i + 1) % ${#accounts[@]}))]}
      fi
    done
  fi
  require_account "$target"
  mkdir -p "$CONF_DIR"
  printf '%s\n' "$target" >"$ACCOUNT_FILE"
  say "New Claude sessions now use '$target'. Running sessions keep their account."
}

account_remove() {
  local label=${1:-}
  [[ -n $label ]] || die_usage "usage: vexos-ai account remove <label>"
  [[ $label != main ]] || die_usage "'main' is Claude's own default sign-in and cannot be removed"
  require_account "$label"
  ! account_in_use "$label" || die_busy "a running Claude session is using '$label'; close it first"
  rm -rf "${ACCOUNTS_DIR:?}/${label:?}"
  if [[ $(read_first_line "$ACCOUNT_FILE" || true) == "$label" ]]; then rm -f "${ACCOUNT_FILE:?}"; fi
  # Files that older versions kept per account for the usage readings.
  rm -f "${CACHE_DIR:?}/usage-$label.json" "${STATE_DIR:?}/warned-$label" \
    "${STATE_DIR:?}/probe-$label.json" "${STATE_DIR:?}/probe-$label.lock"
  say "Removed '$label'."
}

account_rename() {
  local old=${1:-} new=${2:-}
  [[ -n $old && -n $new ]] || die_usage "usage: vexos-ai account rename <label> <new>"
  [[ $old != main ]] || die_usage "'main' is Claude's own default sign-in and cannot be renamed"
  require_account "$old"
  valid_new_label "$new" || die_usage "new label must be 1-32 letters, digits, - or _ (and not 'main' or 'next')"
  [[ ! -e $ACCOUNTS_DIR/$new ]] || die_usage "account '$new' already exists"
  ! account_in_use "$old" || die_busy "a running Claude session is using '$old'; close it first"
  mv "$ACCOUNTS_DIR/$old" "$ACCOUNTS_DIR/$new"
  if [[ $(read_first_line "$ACCOUNT_FILE" || true) == "$old" ]]; then printf '%s\n' "$new" >"$ACCOUNT_FILE"; fi
  say "Renamed '$old' to '$new'."
}

account_mode() {
  local mode=${1:-} t=${2:-}
  if [[ -n $mode ]]; then
    [[ $mode == manual || $mode == auto ]] || die_usage "mode must be manual or auto"
  fi
  if [[ -n $t ]]; then valid_percent "$t" || die_usage "threshold must be a whole number from 1 to 100"; fi
  if [[ -n $mode ]]; then
    mkdir -p "$CONF_DIR"
    printf '%s\n' "$mode" >"$MODE_FILE"
  fi
  if [[ -n $t ]]; then
    mkdir -p "$CONF_DIR"
    printf '%s\n' "$((10#$t))" >"$THRESHOLD_FILE"
  fi
  say "Switching: $(switch_mode) at $(threshold)%. Saved, but it has no effect now: nothing reads usage."
}

account_list() {
  local a active mark
  if [[ -n $JSON ]]; then status_json; return; fi
  active=$(active_account)
  while read -r a; do
    mark=" "
    [[ $a == "$active" ]] && mark="*"
    printf '%s %s\n' "$mark" "$a"
  done < <(list_accounts)
}

# ── Sign-in state ────────────────────────────────────────────────────────────
# What the credentials say, never the tokens: whether the account is signed in
# and its plan label ("Max 5x"). An expired *access* token is routine (Claude
# Code refreshes it when it starts), so only a missing refresh token counts as
# signed out.
account_auth_json() {
  local creds
  creds="$(account_home "$1")/.credentials.json"
  jq -c --argjson nowms "$(($(now_s) * 1000))" '
    (.claudeAiOauth // {}) as $o
    | (($o.subscriptionType // "") | ascii_downcase) as $t
    | first((($o.rateLimitTier // "") | capture("(?<n>[0-9]+)x$")? | .n + "x"), null) as $tier
    | { signedIn: ((($o.refreshToken // "") != "") or ((($o.accessToken // "") != "") and (($o.expiresAt // 0) > $nowms))),
        plan: (if $t == "" then null else ([($t[0:1] | ascii_upcase) + $t[1:], $tier] | map(select(. != null)) | join(" ")) end) }
  ' "$creds" 2>/dev/null || echo '{"signedIn":false,"plan":null}'
}

# ── Status ───────────────────────────────────────────────────────────────────

# The status document, schema 1 (docs/contract.md). Additive keys never bump
# the schema; consumers ignore keys they do not know.
status_json() {
  local a active in_use agent reboot=false rows=()
  active=$(active_account)
  in_use=$(sessions_in_use || true)
  agent=$(default_agent)
  [[ $agent == claude || $agent == opencode ]] || agent=""
  if [[ -e $CURRENT_SYSTEM/sw/bin/vexos-ai && ! -e $BOOTED_SYSTEM/sw/bin/vexos-ai ]]; then reboot=true; fi

  while read -r a; do
    rows+=("$(jq -nc --arg label "$a" --arg active "$active" --arg in_use "$in_use" \
      --argjson auth "$(account_auth_json "$a")" '
      { label: $label, active: ($label == $active), signedIn: $auth.signedIn, plan: $auth.plan,
        inUse: (($in_use | split("\n")) | index($label) != null), usage: null }')")
  done < <(list_accounts)

  # -M: jq colours its output on a terminal, and this runs in VexPortal's VTE.
  printf '%s\n' "${rows[@]}" | jq -scM \
    --arg version "$VERSION" --argjson reboot "$reboot" --arg agent "$agent" \
    --arg workdir "$WORKDIR" --arg active "$active" --arg mode "$(switch_mode)" \
    --argjson threshold "$(threshold)" --arg capture "$(capture_state)" \
    --argjson muted "$(list_muted | jq -Rnc '[inputs]')" '
    { schema: 1, version: $version, installed: true, needsReboot: $reboot,
      agent: (if $agent == "" then null else $agent end), workdir: $workdir,
      accounts: ., activeAccount: $active,
      switching: { mode: $mode, threshold: $threshold },
      crash: { capture: ($capture == "on"), muted: $muted } }'
}

status_text() {
  status_json | jq -r '
    "Assistant:  \(.agent // "not chosen")",
    "Account:    \(.activeAccount) (for new sessions)",
    "Switching:  \(.switching.mode) at \(.switching.threshold)%",
    "Crashes:    capture \(if .crash.capture then "on" else "off" end)\(if (.crash.muted | length) > 0 then ", muted: \(.crash.muted | join(", "))" else "" end)",
    (if .needsReboot then "Reboot:     needed to start the background services" else empty end),
    "Accounts:",
    (.accounts[] | "  \(if .active then "*" else " " end) \(.label)\(if .plan then " (\(.plan))" else "" end)\(if .signedIn then "" else "  not signed in" end)\(if .inUse then "  in use" else "" end)")'
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

# ── Panel (point-and-click) ──────────────────────────────────────────────────

panel() {
  local rows=() a active mark agent out rc label
  have_gui || { account_list; return; }
  while true; do
    rows=()
    active=$(active_account)
    agent=$(default_agent)
    while read -r a; do
      mark=""
      [[ $a == "$active" ]] && mark="●"
      rows+=("$mark" "$a")
    done < <(list_accounts)

    rc=0
    out=$(zenity --list --title="$TITLE" --width=680 --height=380 \
      --text="Assistant: ${agent:-not chosen}   ·   Autoswitch: $(switch_mode) at $(threshold)% (inactive: nothing reads usage)\nClaude accounts (● = used by new sessions):" \
      --column="" --column="Account" \
      --print-column=2 --ok-label="Use selected" --cancel-label="Close" \
      --extra-button="Open assistant" --extra-button="Add account" \
      --extra-button="Remove account" --extra-button="Change AI tool" \
      --extra-button="Toggle autoswitch" \
      "${rows[@]}") || rc=$?

    # A failing command (a dead label, a busy account) must not close the panel.
    case "$rc:$out" in
    0:) ;;
    0:*) (account_use "$out") >/dev/null || true ;;
    1:"Open assistant") launch; return ;;
    1:"Add account")
      label=$(zenity --entry --title="$TITLE" --text="Name for the new Claude account (e.g. work):") || continue
      xdg-terminal-exec vexos-ai account add "$label" &
      return ;;
    1:"Remove account")
      # Extra buttons do not report the selected row, so ask which one.
      label=$(list_accounts | grep -vx main | zenity --list --title="$TITLE" \
        --text="Remove which Claude account? Its sign-in is deleted from this machine." \
        --column="Account" --width=360 --height=300) || continue
      if [[ -n $label ]]; then (account_remove "$label") >/dev/null || true; fi ;;
    1:"Change AI tool") pick_agent || true ;;
    1:"Toggle autoswitch")
      if [[ $(switch_mode) == auto ]]; then account_mode manual >/dev/null; else account_mode auto >/dev/null; fi ;;
    *) return ;;
    esac
  done
}

# ── Theme sync ───────────────────────────────────────────────────────────────
# VexOS themes are GNOME's light/dark scheme plus the role's accent colour
# (modules/gnome-*.nix). Claude Code gets a matching custom theme, which it
# hot-reloads from <home>/themes/. A theme the user picked themselves is left
# alone; only an unset theme or our own is (re)pointed at "custom:vexos".
# OpenCode follows the terminal palette ("theme": "system", modules/ai.nix).

accent_hex() {
  case "$1" in
  teal) echo "#2190a4" ;; green) echo "#3a944a" ;; yellow) echo "#c88800" ;;
  orange) echo "#ed5b00" ;; red) echo "#e62d42" ;; pink) echo "#d56199" ;;
  purple) echo "#9141ac" ;; slate) echo "#6f8396" ;; *) echo "#3584e4" ;;
  esac
}

theme_sync() {
  local scheme accent base hex a home settings tmp tui
  scheme=$(gsettings get org.gnome.desktop.interface color-scheme 2>/dev/null || echo "'prefer-dark'")
  accent=$(gsettings get org.gnome.desktop.interface accent-color 2>/dev/null || echo "'blue'")
  accent=${accent//\'/}
  case "$scheme" in *prefer-light* | *default*) base=light ;; *) base=dark ;; esac
  hex=$(accent_hex "$accent")

  while read -r a; do
    home=$(account_home "$a")
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
  done < <(list_accounts)

  # OpenCode keeps its theme in tui.json; "system" follows the terminal
  # palette, so it tracks light/dark on its own. Only written when absent.
  tui="${XDG_CONFIG_HOME:-$HOME/.config}/opencode/tui.json"
  if [[ ! -e $tui ]]; then
    mkdir -p "${tui%/*}"
    jq -n '{ "$schema": "https://opencode.ai/tui.json", theme: "system" }' >"$tui"
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
      [[ $name == vexos-ai* || $name == claude || $name == opencode ]] && continue
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
    "Let Claude Code or OpenCode help you change and troubleshoot VexOS." 2>/dev/null || true)
  [[ $action == setup ]] && pick_agent && in_new_terminal
}

usage() {
  cat <<'EOF'
Usage: vexos-ai [--non-interactive] [--json] [command]

  (none)                    open your AI assistant in a terminal (in /etc/nixos)
  --prompt "<text>"         open it with a task
  prompt <text…>            the same, words joined
  pick [claude|opencode]    choose the assistant (no argument: ask)
  status [--json] [--refresh]   state; --refresh is accepted and ignored
  panel                     accounts window
  diagnose [unit|rebuild]   collect logs (or reproduce a failed rebuild) and ask what is wrong
  crash <pid> [comm exe sig]  diagnose a core dump (see: coredumpctl list)
  crash-mute [name [on|off]]  mute / unmute / list crash notifications
  crash-mute --off <name>   unmute
  crash-capture [on|off]    watch for crashes at all (no argument: show)
  account list              Claude accounts
  account add <label>       sign in an extra Claude account (opens a browser)
  account use <label|next>  account for new sessions
  account remove <label>
  account rename <label> <new>
  account mode [manual|auto] [threshold]   saved, but unused: nothing reads usage now
  theme                     sync Claude Code's theme with the desktop

With the arguments a command needs it never asks anything. --non-interactive
(or VEXOS_AI_NONINTERACTIVE=1) forbids asking: missing input exits 4.
Exit codes: 0 ok, 1 failed, 2 bad usage, 3 not found, 4 needs interaction, 5 busy.
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
  [[ $# -le 1 ]] || die_usage "usage: vexos-ai pick [claude|opencode]"
  pick_agent "${1:-}" || exit 1
  say "Assistant set to $(agent_title "$(default_agent)")."
  finish
  ;;
status) status_cmd "$@" ;;
panel) panel ;;
diagnose) diagnose "${1:-}" ;;
crash) crash "$@" ;;
crash-mute) crash_mute "$@" ;;
crash-capture)
  [[ $# -le 1 ]] || die_usage "usage: vexos-ai crash-capture [on|off]"
  crash_capture "${1:-}"
  ;;
crash-watch) crash_watch ;;
account)
  sub=${1:-list}
  [[ $# -gt 0 ]] && shift
  case "$sub" in
  list) account_list ;;
  add) account_add "${1:-}" ;;
  use)
    [[ $# -le 1 ]] || die_usage "usage: vexos-ai account use <label|next>"
    account_use "${1:-next}"
    finish
    ;;
  remove)
    [[ $# -le 1 ]] || die_usage "usage: vexos-ai account remove <label>"
    account_remove "${1:-}"
    finish
    ;;
  rename)
    [[ $# -le 2 ]] || die_usage "usage: vexos-ai account rename <label> <new>"
    account_rename "${1:-}" "${2:-}"
    finish
    ;;
  mode)
    [[ $# -le 2 ]] || die_usage "usage: vexos-ai account mode [manual|auto] [threshold]"
    account_mode "${1:-}" "${2:-}"
    finish
    ;;
  *) usage >&2; exit 2 ;;
  esac
  ;;
theme) theme_sync ;;
theme-watch) theme_watch ;;
welcome) welcome ;;
-h | --help | help) usage ;;
*) usage >&2; exit 2 ;;
esac
