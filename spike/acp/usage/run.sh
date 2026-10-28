#!/usr/bin/env bash
# THROWAWAY SPIKE: does Claude Code's /usage work through claude-agent-acp
# 0.88.0 over ACP, and what does it return? Also /status and /cost when the
# adapter lists them, and /usage against opencode acp 1.19.0.
#
# Run from spike/acp on the Linux host:
#   usage/run.sh          run the checks; prints out/usage/report.txt
#   usage/run.sh clean    delete the scratch dir, throwaway login included
#
# HOME, the XDG dirs, CLAUDE_CONFIG_DIR and the agents' working dir all live
# under $SPIKE_SCRATCH (default /tmp/vexos-ai-spike-usage), outside any repo,
# so neither the real home nor this repo's CLAUDE.md or .claude/ is read.
# acp_client.py rejects every permission request. Raw logs go to out/usage/
# (gitignored); every file of a run that matches a credential pattern is
# deleted before anything is reported from it.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../.." && pwd)
SP=$(realpath -m "${SPIKE_SCRATCH:-/tmp/vexos-ai-spike-usage}")
ACP=${ACP_BIN:-/nix/store/w2vz79sc7qflws5hs36955svrsxkrdhr-claude-agent-acp-0.88.0/bin/claude-agent-acp}
OUT=$here/out/usage
REPORT=$OUT/report.txt

case $SP/ in
"$repo"/*) echo "SPIKE_SCRATCH must be outside the repo: $SP" >&2; exit 2 ;;
esac
[[ $(basename "$SP") == vexos-ai-spike* ]] || { echo "SPIKE_SCRATCH's last component must start with vexos-ai-spike: $SP" >&2; exit 2; }

if [[ ${1-} == clean ]]; then
  rm -rf -- "$SP"
  echo "removed $SP"
  exit 0
fi

export HOME=$SP/home XDG_CONFIG_HOME=$SP/home/.config XDG_DATA_HOME=$SP/home/.local/share \
  XDG_CACHE_HOME=$SP/home/.cache XDG_STATE_HOME=$SP/home/.local/state \
  CLAUDE_CONFIG_DIR=$SP/claude-config CLAUDE_AGENT_ACP_PROTOCOL=v1
mkdir -p "$HOME" "$CLAUDE_CONFIG_DIR" "$SP/work" "$OUT"
: >"$REPORT"
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }

# Account identifiers out of anything reported (raw logs stay gitignored).
redact() {
  sed -E 's/[[:alnum:]._%+-]+@[[:alnum:].-]+\.[[:alpha:]]{2,}/<email>/g;
          s/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/<uuid>/g'
}

# Deletes every file of run $1 if any of them looks like it holds a credential.
scan() {
  local f hit=0
  for f in "$OUT/$1".*; do
    [[ -e $f ]] || continue
    if grep -qiE 'sk-ant-|accessToken|refreshToken|"authorization"|bearer [a-z0-9._~+/-]{20,}' "$f"; then hit=1; fi
  done
  if ((hit)); then
    rm -f -- "$OUT/$1".*
    say "$1: credential-like text found; its logs were deleted and nothing is reported from them"
    return 1
  fi
}

# One session: prompts on stdin, one turn each; every permission request rejected.
run() {
  local name=$1
  shift
  rm -f -- "$OUT/$name".*
  "$here/sandbox.sh" "$here/acp_client.py" --log "$OUT/$name.jsonl" --cwd "$SP/work" --timeout 180 -- "$@" \
    >/dev/null 2>&1 || say "$name: client exit $?"
  scan "$name"
}

# The adapter's command list (the last available_commands_update), name and description.
commands() {
  jq -cs '[.[] | select(.kind == "in" and .msg.params.update.sessionUpdate? == "available_commands_update")
           | .msg.params.update.availableCommands] | last // [] | map({name, description})' "$OUT/$1.jsonl"
}

# Per turn: elapsed time, response, update kinds, usage and mode reports, tool
# calls, permission requests, and the text exactly as received.
turns() {
  jq -s '. as $all
    | ($all | map(select(.kind == "in" and .msg.result.sessionId?)) | first | .msg.result
       | (.modes.currentModeId // ([.configOptions[]? | select(.id == "mode") | .currentValue] | join(",")))) as $start
    | $all[] | select(.kind == "out" and .msg.method == "session/prompt") as $r
    | ($all | map(select(.kind == "in" and .msg.id == $r.msg.id and (.msg.result != null or .msg.error != null))) | first) as $resp
    | ($resp.t // 1e12) as $end
    | ($all | map(select(.kind == "in" and .msg.method == "session/update" and .t >= $r.t and .t <= $end)
                  | .msg.params.update)) as $ups
    | { prompt: $r.msg.params.prompt[0].text,
        session_start_mode: $start,
        elapsed_s: (if $resp then ((($resp.t - $r.t) * 1000) | round) / 1000 else "no response" end),
        response: ($resp.msg.result // $resp.msg.error),
        update_kinds: ($ups | map(.sessionUpdate) | group_by(.) | map({(.[0]): length}) | add),
        usage_updates: ($ups | map(select(.sessionUpdate == "usage_update") | {used, size, cost, _meta})),
        mode_reports: ($ups | map(select(.sessionUpdate == "current_mode_update" or .sessionUpdate == "config_option_update"))),
        tool_calls: ($ups | map(select(.sessionUpdate == "tool_call" or .sessionUpdate == "tool_call_update"))),
        permission_requests: ($all | map(select(.kind == "in" and .msg.method == "session/request_permission"
                                                and .t >= $r.t and .t <= $end)) | length),
        text: ($ups | map(select(.sessionUpdate == "agent_message_chunk") | .content.text // "") | join("")) }' \
    "$OUT/$1.jsonl"
}

report() {
  say "== $1"
  say "available commands: $(commands "$1" | redact)"
  turns "$1" | redact | tee -a "$REPORT"
}

# 7. OpenCode: no login needed (default free model, as in the earlier spike).
oc_version=$(opencode --version 2>/dev/null || echo missing)
if [[ $oc_version == 1.19.0 ]]; then
  echo "/usage" | run opencode-usage opencode acp && report opencode-usage
else
  say "opencode-usage: NOT RUN, opencode on PATH is '$oc_version', not 1.19.0"
fi

# 1-6. Claude: needs a throwaway Claude.ai login in the scratch config dir.
# Claude itself reports it (exit 1 when not signed in; an API key in the
# environment would also count, hence the authMethod check); this script
# never opens the login file.
if ! claude auth status 2>/dev/null | jq -e '.authMethod == "claude.ai"' >/dev/null 2>&1; then
  say "claude: NOT RUN, no Claude login in $CLAUDE_CONFIG_DIR."
  say "  Sign in once, through Anthropic's own flow, into that throwaway dir:"
  say "    HOME=$HOME CLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR claude auth login"
  say "  then run usage/run.sh again. 'usage/run.sh clean' deletes it afterwards."
  exit 3
fi
[[ -x $ACP ]] || { say "claude: NOT RUN, $ACP not found (set ACP_BIN)"; exit 3; }

# /usage, then a plain model turn in the same session as the timing and
# context-token baseline.
printf '%s\n' "/usage" "Reply with the single word ok." | run claude-usage "$ACP" && report claude-usage

# 5. The alternatives, each in its own session, only if the adapter listed them.
if [[ -e $OUT/claude-usage.jsonl ]]; then
  for c in status cost; do
    if commands claude-usage | jq -e --arg c "$c" 'any(.[]; .name == $c)' >/dev/null; then
      echo "/$c" | run "claude-$c" "$ACP" && report "claude-$c"
    else
      say "claude-$c: not run, /$c is not in the adapter's command list"
    fi
  done
fi
say "raw logs: $OUT (gitignored). Check report.txt for organisation names before sharing it."
