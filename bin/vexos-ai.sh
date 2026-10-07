# vexos-ai — VexOS AI assistant launcher (modules/ai.nix).
#
# Opens the user's chosen coding agent (Claude Code or OpenCode) in a terminal,
# working in /etc/nixos, with the VexOS skills (files/ai/skills) installed in
# ~/.claude/skills — both agents read that directory. Also owns the pieces
# around it: first-run picker, multiple Claude accounts, subscription usage
# warnings, theme sync, and "diagnose with AI" for crashes, failed units and
# failed rebuilds.
#
# Agents always start in their default, asking permission mode. Applying a
# change (nixos-rebuild switch/boot, just rebuild) is denied to them by the
# system-wide agent configs written by modules/ai.nix; the user rebuilds.

CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/vexos/ai"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/vexos/ai"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/vexos/ai"
ACCOUNTS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/vexos/ai/claude-accounts"
AGENT_FILE="$CONF_DIR/agent"
ACCOUNT_FILE="$CONF_DIR/claude-account"
MODE_FILE="$CONF_DIR/mode"
THRESHOLD_FILE="$CONF_DIR/threshold"
MUTE_DIR="$CONF_DIR/crash-ignore"
WORKDIR=/etc/nixos
USAGE_ENDPOINT=https://api.anthropic.com/api/oauth/usage
USAGE_MAX_AGE=300   # seconds; Anthropic rate-limits the usage endpoint readily
TITLE="VexOS Assistant"

# ── Helpers ──────────────────────────────────────────────────────────────────

die() { echo "vexos-ai: $*" >&2; exit 1; }

notify() { notify-send -a "$TITLE" -i utilities-terminal "$@" 2>/dev/null || true; }

have_gui() { [[ -n ${WAYLAND_DISPLAY:-}${DISPLAY:-} ]]; }

in_terminal() { [[ -t 0 && -t 1 ]]; }

# Re-run this command inside a terminal window. Arguments travel as argv,
# never through a shell string, so a prompt or process name cannot be reparsed.
in_new_terminal() { exec xdg-terminal-exec vexos-ai "$@"; }

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
    find "$ACCOUNTS_DIR" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort
  fi
}

valid_label() { [[ $1 =~ ^[A-Za-z0-9_-]{1,32}$ && $1 != main ]]; }

threshold() {
  local t
  t=$(read_first_line "$THRESHOLD_FILE" || true)
  if [[ $t =~ ^[0-9]{1,3}$ ]] && ((t >= 1 && t <= 100)); then echo "$t"; else echo 95; fi
}

switch_mode() {
  if [[ $(read_first_line "$MODE_FILE" || true) == auto ]]; then echo auto; else echo manual; fi
}

# ── Picker ───────────────────────────────────────────────────────────────────

pick_agent() {
  local choice=""
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
  mkdir -p "$CONF_DIR"
  printf '%s\n' "$choice" >"$AGENT_FILE"
}

# ── Launch ───────────────────────────────────────────────────────────────────

launch() {
  local prompt=${1:-} agent acct
  agent=$(default_agent)
  if [[ -z $agent ]]; then
    pick_agent || exit 1
    agent=$(default_agent)
  fi

  if ! in_terminal; then
    if [[ -n $prompt ]]; then in_new_terminal --prompt "$prompt"; else in_new_terminal; fi
  fi

  cd "$WORKDIR" 2>/dev/null || cd "$HOME"

  case "$agent" in
  claude)
    acct=$(active_account)
    if [[ $acct != main ]]; then
      CLAUDE_CONFIG_DIR=$(account_home "$acct")
      export CLAUDE_CONFIG_DIR
    fi
    if [[ -n $prompt ]]; then exec claude "$prompt"; else exec claude; fi
    ;;
  opencode)
    if [[ -n $prompt ]]; then exec opencode --prompt "$prompt"; else exec opencode; fi
    ;;
  *) die "unknown assistant '$agent' in $AGENT_FILE (run: vexos-ai pick)" ;;
  esac
}

# ── Accounts (Claude) ────────────────────────────────────────────────────────

account_add() {
  local label=${1:-} home
  valid_label "$label" || die "account label must be 1-32 letters, digits, - or _ (and not 'main')"
  home=$(account_home "$label")
  [[ -e $home ]] && die "account '$label' already exists"
  if ! in_terminal; then in_new_terminal account add "$label"; fi

  mkdir -p "$home"
  chmod 700 "$home"
  mkdir -p "$HOME/.claude/skills"
  ln -sfn "$HOME/.claude/skills" "$home/skills"
  theme_sync >/dev/null 2>&1 || true

  echo "Signing in a new Claude account: $label"
  echo "Tip: if your browser is already signed in to another Claude account,"
  echo "open the sign-in link in a private window."
  echo
  if CLAUDE_CONFIG_DIR="$home" claude auth login; then
    echo
    echo "Added '$label'. Switch to it with: vexos-ai account use $label"
  else
    rm -rf "$home"
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
  [[ $target == main || -d $ACCOUNTS_DIR/$target ]] || die "no account named '$target'"
  mkdir -p "$CONF_DIR"
  printf '%s\n' "$target" >"$ACCOUNT_FILE"
  echo "New Claude sessions now use '$target'. Running sessions keep their account."
}

account_remove() {
  local label=${1:-}
  valid_label "$label" || die "cannot remove '$label'"
  [[ -d $ACCOUNTS_DIR/$label ]] || die "no account named '$label'"
  rm -rf "${ACCOUNTS_DIR:?}/$label"
  [[ $(read_first_line "$ACCOUNT_FILE" || true) == "$label" ]] && rm -f "$ACCOUNT_FILE"
  rm -f "$CACHE_DIR/usage-$label.json" "$STATE_DIR/warned-$label"
  echo "Removed '$label'."
}

account_mode() {
  local mode=${1:-} t=${2:-}
  if [[ -n $mode ]]; then
    [[ $mode == manual || $mode == auto ]] || die "mode must be manual or auto"
    mkdir -p "$CONF_DIR"
    printf '%s\n' "$mode" >"$MODE_FILE"
  fi
  if [[ -n $t ]]; then
    if ! [[ $t =~ ^[0-9]{1,3}$ ]] || ((t < 1 || t > 100)); then die "threshold must be 1-100"; fi
    printf '%s\n' "$t" >"$THRESHOLD_FILE"
  fi
  if [[ $(switch_mode) == auto ]]; then
    echo "Near $(threshold)% of a limit, new sessions switch to the account with the most headroom."
  else
    echo "Near $(threshold)% of a limit you get a notification; switching is up to you."
  fi
}

account_list() {
  local a active rec mark
  active=$(active_account)
  while read -r a; do
    rec=$(usage_of "$a")
    mark=" "
    [[ $a == "$active" ]] && mark="*"
    printf '%s %-16s %s\n' "$mark" "$a" \
      "$(jq -r 'if .ok then "session \(.session|floor)%  weekly \(.weekly|floor)%" else "usage unknown" end' <<<"$rec")"
  done < <(list_accounts)
}

# ── Usage (Claude subscriptions) ─────────────────────────────────────────────
# Anthropic's OAuth usage endpoint is the one Claude Code's own /usage reads.
# It is undocumented, so every failure degrades to {"ok":false} rather than an
# error. The access token is passed to curl on stdin (never argv, which other
# users can read) and goes nowhere but the Authorization header.

probe_usage() {
  local home=$1 token expires now
  token=$(jq -r '.claudeAiOauth.accessToken // empty' "$home/.credentials.json" 2>/dev/null || true)
  expires=$(jq -r '.claudeAiOauth.expiresAt // 0' "$home/.credentials.json" 2>/dev/null || echo 0)
  now=$(($(date +%s) * 1000))
  if [[ -z $token || ! $expires =~ ^[0-9]+$ ]] || ((expires < now)); then
    echo '{"ok":false,"why":"not signed in, or sign-in expired (open the assistant to refresh it)"}'
    return
  fi
  curl -fsS --max-time 10 -H @- \
    -H "anthropic-beta: oauth-2025-04-20" -H "Accept: application/json" \
    "$USAGE_ENDPOINT" <<<"Authorization: Bearer $token" 2>/dev/null |
    jq -c '
      def num: if type == "number" then . elif type == "string" then (tonumber? // null) else null end;
      (.seven_day_oauth_apps // .seven_day) as $w
      | [(.five_hour.utilization | num), ($w.utilization | num)] as $v
      | ([$v[] | select(. != null and . >= 1)] | length > 0) as $percent
      | def pct: if . == null then null elif $percent then . else . * 100 end;
      { ok: ($v | any(. != null)),
        session: (($v[0] | pct) // 0), sessionReset: (.five_hour.resets_at // ""),
        weekly: (($v[1] | pct) // 0), weeklyReset: ($w.resets_at // "") }' 2>/dev/null ||
    echo '{"ok":false,"why":"usage endpoint unavailable"}'
}

# Cached per account for USAGE_MAX_AGE seconds; pass "force" to re-probe.
usage_of() {
  local acct=$1 force=${2:-} file rec age
  file="$CACHE_DIR/usage-$acct.json"
  if [[ -z $force && -f $file ]]; then
    age=$(($(date +%s) - $(stat -c %Y "$file")))
    if ((age < USAGE_MAX_AGE)); then cat "$file"; return; fi
  fi
  rec=$(probe_usage "$(account_home "$acct")")
  mkdir -p "$CACHE_DIR"
  printf '%s\n' "$rec" >"$file.tmp" && mv "$file.tmp" "$file"
  echo "$rec"
}

peak() { jq -r 'if .ok then ([.session, .weekly] | max | floor) else -1 end' <<<"$1"; }

# Run by vexos-ai-usage.timer. Warns once per account per limit window; in
# auto mode also moves new sessions to the account with the most headroom.
usage_check() {
  local t active rec p key stamp best best_p a ap
  [[ $(default_agent) == claude ]] || return 0
  t=$(threshold)
  active=$(active_account)
  rec=$(usage_of "$active" force)
  p=$(peak "$rec")
  ((p >= t)) || return 0

  key=$(jq -r '"\(.sessionReset)|\(.weeklyReset)"' <<<"$rec")
  stamp="$STATE_DIR/warned-$active"
  [[ $(read_first_line "$stamp" || true) == "$key" ]] && return 0
  mkdir -p "$STATE_DIR"
  printf '%s\n' "$key" >"$stamp"

  if [[ $(switch_mode) == auto ]]; then
    best="" best_p=101
    while read -r a; do
      [[ $a == "$active" ]] && continue
      ap=$(peak "$(usage_of "$a")")
      if ((ap >= 0 && ap < t && ap < best_p)); then best=$a best_p=$ap; fi
    done < <(list_accounts)
    if [[ -n $best ]]; then
      account_use "$best" >/dev/null
      notify -u normal "Claude account '$active' is at ${p}%" \
        "New sessions now use '$best' (${best_p}%). Running sessions are unchanged."
      return 0
    fi
  fi
  notify -u critical "Claude account '$active' is at ${p}% of its limit" \
    "Open VexOS Assistant → Accounts & usage to switch accounts."
}

# ── Panel (point-and-click) ──────────────────────────────────────────────────

panel() {
  local rows=() a active rec mark agent out rc label
  have_gui || { account_list; return; }
  while true; do
    rows=()
    active=$(active_account)
    agent=$(default_agent)
    while read -r a; do
      rec=$(usage_of "$a")
      mark=""
      [[ $a == "$active" ]] && mark="●"
      rows+=("$mark" "$a"
        "$(jq -r 'if .ok then "\(.session|floor)%" else "—" end' <<<"$rec")"
        "$(jq -r 'if .ok then "\(.weekly|floor)%" else (.why // "unknown") end' <<<"$rec")")
    done < <(list_accounts)

    rc=0
    out=$(zenity --list --title="$TITLE" --width=680 --height=380 \
      --text="Assistant: ${agent:-not chosen}   ·   Autoswitch: $(switch_mode) at $(threshold)%\nClaude accounts (● = used by new sessions):" \
      --column="" --column="Account" --column="Session (5h)" --column="Weekly" \
      --print-column=2 --ok-label="Use selected" --cancel-label="Close" \
      --extra-button="Open assistant" --extra-button="Add account" \
      --extra-button="Remove account" --extra-button="Change AI tool" \
      --extra-button="Toggle autoswitch" \
      "${rows[@]}") || rc=$?

    case "$rc:$out" in
    0:) ;;
    0:*) account_use "$out" >/dev/null ;;
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
      [[ -n $label ]] && account_remove "$label" >/dev/null ;;
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
  if ! in_terminal; then in_new_terminal diagnose ${unit:+"$unit"}; fi
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
  [[ $pid =~ ^[0-9]+$ ]] || die "usage: vexos-ai crash <pid> [comm] [exe] [signal]   (see: coredumpctl list)"
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

crash_mute() {
  local name=${1:-} state=${2:-on}
  if [[ -z $name ]]; then
    [[ -d $MUTE_DIR ]] && find "$MUTE_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort
    return 0
  fi
  name=${name##*/}
  [[ -n $name && $name != . && $name != .. ]] || die "not a program name"
  mkdir -p "$MUTE_DIR"
  if [[ $state == off ]]; then
    rm -f "$MUTE_DIR/$name"
    echo "Crash notifications for '$name' are on again."
  else
    touch "$MUTE_DIR/$name"
    echo "Crash notifications for '$name' are muted. Undo: vexos-ai crash-mute '$name' off"
  fi
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
Usage: vexos-ai [command]

  (none)                    open your AI assistant in a terminal (in /etc/nixos)
  --prompt "<text>"         open it with a task
  pick                      choose Claude Code or OpenCode
  panel                     accounts & usage window
  diagnose [unit|rebuild]   collect logs (or reproduce a failed rebuild) and ask what is wrong
  crash <pid> [comm exe sig]  diagnose a core dump (see: coredumpctl list)
  crash-mute [name] [off]   mute / unmute / list crash notifications
  account list              Claude accounts and their usage
  account add <label>       sign in an extra Claude account
  account use <label|next>  account for new sessions
  account remove <label>
  account mode [manual|auto] [threshold]   warn only, or also switch, near a limit
  usage [account]           print usage JSON (cached 5 min)
  theme                     sync Claude Code's theme with the desktop
EOF
}

# ── Dispatch ─────────────────────────────────────────────────────────────────

cmd=${1:-}
[[ $# -gt 0 ]] && shift
case "$cmd" in
"") launch ;;
--prompt) launch "${1:?--prompt needs text}" ;;
pick) pick_agent ;;
panel) panel ;;
diagnose) diagnose "${1:-}" ;;
crash) crash "$@" ;;
crash-mute) crash_mute "$@" ;;
crash-watch) crash_watch ;;
account)
  sub=${1:-list}
  [[ $# -gt 0 ]] && shift
  case "$sub" in
  list) account_list ;;
  add) account_add "${1:-}" ;;
  use) account_use "${1:-next}" ;;
  remove) account_remove "${1:-}" ;;
  mode) account_mode "$@" ;;
  *) usage; exit 1 ;;
  esac
  ;;
usage) usage_of "${1:-$(active_account)}" force ;;
usage-check) usage_check ;;
theme) theme_sync ;;
theme-watch) theme_watch ;;
welcome) welcome ;;
-h | --help | help) usage ;;
*) usage; exit 1 ;;
esac
