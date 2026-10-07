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
USAGE_ENDPOINT=https://api.anthropic.com/api/oauth/usage
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

notify() { notify-send -a "$TITLE" -i utilities-terminal "$@" 2>/dev/null || true; }

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

iso_utc() { date -u "$@"; }
now_s() { date +%s; }
ISO_FMT=+%Y-%m-%dT%H:%M:%S+00:00

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
  rm -f "${CACHE_DIR:?}/usage-$label.json" "${STATE_DIR:?}/warned-$label" \
    "${STATE_DIR:?}/probe-$label.json" "${STATE_DIR:?}/probe-$label.lock"
  say "Removed '$label'."
}

move_if_exists() { if [[ -e $1 ]]; then mv "$1" "$2"; fi; }

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
  move_if_exists "$CACHE_DIR/usage-$old.json" "$CACHE_DIR/usage-$new.json"
  move_if_exists "$STATE_DIR/warned-$old" "$STATE_DIR/warned-$new"
  move_if_exists "$STATE_DIR/probe-$old.json" "$STATE_DIR/probe-$new.json"
  rm -f "${STATE_DIR:?}/probe-$old.lock"
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
  if [[ $(switch_mode) == auto ]]; then
    say "Near $(threshold)% of a limit, new sessions switch to the account with the most headroom."
  else
    say "Near $(threshold)% of a limit you get a notification; switching is up to you."
  fi
}

account_list() {
  local a active rec mark
  if [[ -n $JSON ]]; then status_json; return; fi
  active=$(active_account)
  while read -r a; do
    usage_refresh "$a" || true
    rec=$(account_usage_json "$a")
    mark=" "
    [[ $a == "$active" ]] && mark="*"
    printf '%s %-16s %s\n' "$mark" "$a" \
      "$(jq -r 'if .ok then "session \(.session|floor)%  weekly \(.weekly|floor)%\(if .stale then "  (last known)" else "" end)" else "usage unknown" end' <<<"$rec")"
  done < <(list_accounts)
}

# ── Usage (Claude subscriptions) ─────────────────────────────────────────────
# Anthropic's OAuth usage endpoint is the one Claude Code's own /usage reads.
# It is undocumented and rate-limited, so it is treated as hostile:
#   - one probe per account at a time (flock), whoever asks: timer, CLI, UI;
#   - at most every 10 min for the active account, 3 min when it is within 15
#     points of the threshold, 60 min for the others; `--refresh` skips the
#     interval but never a backoff, and never runs twice within 15 s;
#   - 429 honours Retry-After, any other failure backs off 1, 2, 4 … 60 min;
#     a transport error retries in a minute and is not counted as a failure;
#   - a failed probe keeps the last good numbers, marked stale, with a short
#     human reason; an unknown payload is "usage format changed", not a crash.
# The access token is read from .credentials.json, piped to curl's stdin (never
# argv, which other users can read) and goes nowhere but the Authorization
# header. It is never logged, cached, written to state or printed.
#
# The cache file keeps the original keys (ok session weekly sessionReset
# weeklyReset why — released VexPortal reads them, and `why` must stay a
# string) and adds fetchedAt plan scoped stale.

# jq helpers: when a window's reset time has passed, its percentage is 0.
# shellcheck disable=SC2016 # jq variables, not shell ones
USAGE_JQ_DEFS='
def epoch: if type == "string" and . != "" then (sub("\\.[0-9]+"; "") | sub("\\+00:00$"; "Z") | (try fromdateiso8601 catch null)) else null end;
def live($now; $at): (($at | epoch)) as $e | if $e != null and $e <= $now then 0 else . end;
def nz: if . == "" then null else . end;
'

probe_state_file() { echo "$STATE_DIR/probe-$1.json"; }

state_num() {
  local v
  v=$(jq -r --arg k "$2" '(.[$k] // 0) | if type == "number" then floor else 0 end' "$1" 2>/dev/null || true)
  echo "${v:-0}"
}

write_state() { # acct nextAt failures lastAt
  local file
  file=$(probe_state_file "$1")
  mkdir -p "$STATE_DIR"
  jq -nc --argjson n "$2" --argjson f "$3" --argjson l "$4" '{nextAt: $n, failures: $f, lastAt: $l}' >"$file.tmp" &&
    mv "$file.tmp" "$file"
}

write_cache() { # acct json
  local file="$CACHE_DIR/usage-$1.json"
  mkdir -p "$CACHE_DIR"
  printf '%s\n' "$2" >"$file.tmp" && mv "$file.tmp" "$file"
}

# The cache as the status document's `usage` object, or null before any reading.
account_usage_json() {
  local file="$CACHE_DIR/usage-$1.json" mtime
  if [[ ! -f $file ]]; then echo null; return; fi
  mtime=$(iso_utc -d "@$(stat -c %Y "$file")" "$ISO_FMT")
  jq -c --argjson now "$(now_s)" --arg mtime "$mtime" "$USAGE_JQ_DEFS"'
    . as $c
    | ($c.ok // false) as $ok
    | { ok: $ok,
        session: (if $ok then (($c.session // 0) | live($now; $c.sessionReset)) else null end),
        sessionResetsAt: (if $ok then ($c.sessionReset | nz) else null end),
        weekly: (if $ok then (($c.weekly // 0) | live($now; $c.weeklyReset)) else null end),
        weeklyResetsAt: (if $ok then ($c.weeklyReset | nz) else null end),
        scoped: (if $ok then [($c.scoped // [])[] | . as $s
                              | { label, percent: (.percent | live($now; $s.resetsAt)), resetsAt: (.resetsAt | nz) }] else [] end),
        fetchedAt: ($c.fetchedAt // (if $ok then $mtime else null end)),
        stale: ($c.stale // false),
        why: (($c.why // "") | nz) }' "$file" 2>/dev/null || echo null
}

peak_of() {
  account_usage_json "$1" | jq -r 'if .ok then ([.session, .weekly] | max | floor) else -1 end'
}

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

backoff_delay() { # failures -> seconds: 60, 120, 240 … capped at 3600
  local d=$((60 * (1 << ($1 > 7 ? 6 : $1 - 1))))
  echo $((d > 3600 ? 3600 : d))
}

# Keep the last good numbers (marked stale), or record that there are none.
write_degraded() { # acct why
  local file="$CACHE_DIR/usage-$1.json" old=null mtime=""
  if [[ -f $file ]]; then
    old=$(cat "$file")
    mtime=$(iso_utc -d "@$(stat -c %Y "$file")" "$ISO_FMT")
  fi
  write_cache "$1" "$(jq -c --arg why "$2" --arg mtime "$mtime" '
    if type == "object" and .ok == true then
      .why = $why | .stale = true | .fetchedAt = (.fetchedAt // $mtime)
    else
      { ok: false, session: 0, sessionReset: "", weekly: 0, weeklyReset: "", why: $why,
        scoped: [], plan: (if type == "object" then .plan // "" else "" end), stale: false }
    end' <<<"$old" 2>/dev/null ||
    jq -nc --arg why "$2" '{ok: false, session: 0, sessionReset: "", weekly: 0, weeklyReset: "", why: $why, scoped: [], plan: "", stale: false}')"
}

# The payload as a cache record. Fails on a shape it does not know.
parse_usage() { # body-file plan
  jq -ce --arg fetched "$(iso_utc "$ISO_FMT")" --arg plan "$2" '
    def num: if type == "number" then . elif type == "string" then (tonumber? // null) else null end;
    if type != "object" then error("shape") else . end
    | (.seven_day_oauth_apps // .seven_day) as $w
    | [to_entries[]
        | select((.key | startswith("seven_day_")) and .key != "seven_day_oauth_apps"
                 and (.value | type) == "object" and (.value.utilization | num) != null)
        | { label: ((.key | ltrimstr("seven_day_") | split("_") | map((.[0:1] | ascii_upcase) + .[1:]) | join(" ")) + " Weekly"),
            raw: (.value.utilization | num), resetsAt: (.value.resets_at // "") }] as $sc
    | [(.five_hour.utilization | num), ($w.utilization | num)] as $v
    | (([$v[], $sc[].raw] | map(select(. != null and . >= 1)) | length) > 0) as $percent
    | def pct: if . == null then null elif $percent then . else . * 100 end;
      if ($v | any(. != null)) | not then error("shape") else
      { ok: true,
        session: (($v[0] | pct) // 0), sessionReset: (.five_hour.resets_at // ""),
        weekly: (($v[1] | pct) // 0), weeklyReset: ($w.resets_at // ""),
        why: "", fetchedAt: $fetched, plan: $plan,
        scoped: [$sc[] | {label, percent: (.raw | pct), resetsAt}], stale: false } end
  ' "$1"
}

probe_locked() { # acct [force]
  local acct=$1 force=${2:-} sfile now next last fails home creds token expires
  local hdr body code rc=0 retry delay rec plan interval=3600 p
  sfile=$(probe_state_file "$acct")
  now=$(now_s)
  next=$(state_num "$sfile" nextAt)
  last=$(state_num "$sfile" lastAt)
  fails=$(state_num "$sfile" failures)

  ((now >= next)) || return 0 # backing off — --refresh does not override that
  if [[ -n $force ]]; then
    ((now - last >= 15)) || return 0
  else
    if [[ $acct == "$(active_account)" ]]; then
      interval=600
      p=$(peak_of "$acct")
      if ((p >= 0 && p >= $(threshold) - 15)); then interval=180; fi
    fi
    ((now - last >= interval)) || return 0
  fi

  home=$(account_home "$acct")
  creds="$home/.credentials.json"
  token=$(jq -r '.claudeAiOauth.accessToken // empty' "$creds" 2>/dev/null || true)
  expires=$(jq -r '.claudeAiOauth.expiresAt // 0' "$creds" 2>/dev/null || echo 0)
  plan=$(account_auth_json "$acct" | jq -r '.plan // ""')
  if [[ -z $token ]]; then
    write_degraded "$acct" "not signed in"
    return 0
  fi
  if [[ ! $expires =~ ^[0-9]+$ ]] || ((expires < now * 1000)); then
    token=""
    write_degraded "$acct" "start Claude Code to refresh it"
    return 0
  fi

  hdr=$(mktemp)
  body=$(mktemp)
  printf 'Authorization: Bearer %s\n' "$token" |
    curl -sS --max-time 10 -H @- \
      -H "anthropic-beta: oauth-2025-04-20" -H "Accept: application/json" \
      -D "$hdr" -o "$body" -w '%{http_code}' "$USAGE_ENDPOINT" >"$body.code" 2>/dev/null || rc=$?
  token=""
  code=$(cat "$body.code" 2>/dev/null || true)

  if ((rc != 0)); then
    # No route yet (just logged in, resuming): retry soon, and do not count it.
    write_state "$acct" $((now + 60)) "$fails" "$now"
    write_degraded "$acct" "cannot reach the usage service; retrying"
  else
    case "$code" in
    200)
      if rec=$(parse_usage "$body" "$plan" 2>/dev/null); then
        write_state "$acct" 0 0 "$now"
        write_cache "$acct" "$rec"
      else
        fails=$((fails + 1))
        write_state "$acct" $((now + $(backoff_delay "$fails"))) "$fails" "$now"
        write_degraded "$acct" "usage format changed"
      fi
      ;;
    429)
      fails=$((fails + 1))
      retry=$(grep -i '^retry-after:' "$hdr" | head -n 1 | tr -d '\r ' | cut -d: -f2 || true)
      if [[ $retry =~ ^[0-9]{1,9}$ ]]; then
        delay=$((10#$retry < 1 ? 1 : (10#$retry > 86400 ? 86400 : 10#$retry)))
      else
        delay=$(backoff_delay "$fails")
      fi
      write_state "$acct" $((now + delay)) "$fails" "$now"
      write_degraded "$acct" "rate limited; retrying in $(((delay + 59) / 60)) min"
      ;;
    401 | 403)
      fails=$((fails + 1))
      write_state "$acct" $((now + $(backoff_delay "$fails"))) "$fails" "$now"
      write_degraded "$acct" "sign-in was rejected; start Claude Code to refresh it"
      ;;
    *)
      fails=$((fails + 1))
      write_state "$acct" $((now + $(backoff_delay "$fails"))) "$fails" "$now"
      write_degraded "$acct" "usage service unavailable (HTTP ${code:-?})"
      ;;
    esac
  fi
  rm -f "${body:?}.code" "${hdr:?}" "${body:?}"
}

# Refresh one account's reading if it is due. Single-flight: with `force`
# (--refresh, `usage`) wait for a probe already running, otherwise skip.
usage_refresh() { # acct [force]
  local acct=$1 force=${2:-}
  mkdir -p "$STATE_DIR" "$CACHE_DIR"
  (
    if [[ -n $force ]]; then flock -w 30 9 || exit 0; else flock -n 9 || exit 0; fi
    probe_locked "$acct" "$force"
  ) 9>"$STATE_DIR/probe-$acct.lock" || true
}

# `usage [account]`: the cache record, as it has always been printed.
usage_cmd() {
  local acct=${1:-$(active_account)}
  require_account "$acct"
  usage_refresh "$acct" force
  if [[ -f $CACHE_DIR/usage-$acct.json ]]; then
    cat "$CACHE_DIR/usage-$acct.json"
  else
    echo '{"ok":false,"why":"no reading yet"}'
  fi
}

# Run by vexos-ai-usage.timer. Warns once per account per limit window; in
# auto mode also moves new sessions to the account with the most headroom.
usage_check() {
  local t active p key stamp best best_p a ap usage
  [[ $(default_agent) == claude ]] || return 0
  t=$(threshold)
  active=$(active_account)
  while read -r a; do usage_refresh "$a" || true; done < <(list_accounts)
  p=$(peak_of "$active")
  ((p >= t)) || return 0

  usage=$(account_usage_json "$active")
  key=$(jq -r '"\(.sessionResetsAt)|\(.weeklyResetsAt)"' <<<"$usage")
  stamp="$STATE_DIR/warned-$active"
  [[ $(read_first_line "$stamp" || true) == "$key" ]] && return 0
  mkdir -p "$STATE_DIR"
  printf '%s\n' "$key" >"$stamp"

  if [[ $(switch_mode) == auto ]]; then
    best="" best_p=101
    while read -r a; do
      [[ $a == "$active" ]] && continue
      ap=$(peak_of "$a")
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
      --argjson auth "$(account_auth_json "$a")" --argjson usage "$(account_usage_json "$a")" '
      { label: $label, active: ($label == $active), signedIn: $auth.signedIn, plan: $auth.plan,
        inUse: (($in_use | split("\n")) | index($label) != null), usage: $usage }')")
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
    def pct: if . == null then "?" else "\(floor)%" end;
    "Assistant:  \(.agent // "not chosen")",
    "Account:    \(.activeAccount) (for new sessions)",
    "Switching:  \(.switching.mode) at \(.switching.threshold)%",
    "Crashes:    capture \(if .crash.capture then "on" else "off" end)\(if (.crash.muted | length) > 0 then ", muted: \(.crash.muted | join(", "))" else "" end)",
    (if .needsReboot then "Reboot:     needed to start the background services" else empty end),
    "Accounts:",
    (.accounts[] | "  \(if .active then "*" else " " end) \(.label)\(if .plan then " (\(.plan))" else "" end)\(if .signedIn then "" else "  not signed in" end)\(if .inUse then "  in use" else "" end)",
      "      " + (if .usage == null then "no usage reading yet"
        elif .usage.ok then "session \(.usage.session | pct)  weekly \(.usage.weekly | pct)\(if .usage.stale then "  (last known)" else "" end)"
        else "usage unknown\(if .usage.why then ": \(.usage.why)" else "" end)" end))'
}

status_cmd() {
  local refresh="" a
  while (($#)); do
    case "$1" in
    --refresh) refresh=1 ;;
    *) die_usage "usage: vexos-ai status [--json] [--refresh]" ;;
    esac
    shift
  done
  if [[ -n $refresh ]]; then
    while read -r a; do usage_refresh "$a" force; done < <(list_accounts)
  fi
  if [[ -n $JSON ]]; then status_json; else status_text; fi
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
      usage_refresh "$a" || true
      rec=$(account_usage_json "$a")
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
  status [--json] [--refresh]   state; --refresh re-reads usage (rate-limited)
  panel                     accounts & usage window
  diagnose [unit|rebuild]   collect logs (or reproduce a failed rebuild) and ask what is wrong
  crash <pid> [comm exe sig]  diagnose a core dump (see: coredumpctl list)
  crash-mute [name [on|off]]  mute / unmute / list crash notifications
  crash-mute --off <name>   unmute
  crash-capture [on|off]    watch for crashes at all (no argument: show)
  account list              Claude accounts and their usage
  account add <label>       sign in an extra Claude account (opens a browser)
  account use <label|next>  account for new sessions
  account remove <label>
  account rename <label> <new>
  account mode [manual|auto] [threshold]   warn only, or also switch, near a limit
  usage [account]           print usage JSON (re-read, rate-limited)
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
usage) usage_cmd "${1:-}" ;;
usage-check) usage_check ;;
theme) theme_sync ;;
theme-watch) theme_watch ;;
welcome) welcome ;;
-h | --help | help) usage ;;
*) usage >&2; exit 2 ;;
esac
