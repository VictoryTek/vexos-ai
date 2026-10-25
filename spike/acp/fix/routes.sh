#!/usr/bin/env bash
# Re-runnable route checks. $1 = tag (baseline|fixed). Runs from spike/acp.
# THROWAWAY SPIKE: fixed paths without spaces, so word splitting is fine.
# shellcheck disable=SC2086
set -u
SP=${SPIKE_SCRATCH:-/tmp/vexos-ai-spike-fix}   # scratch dirs, outside any repo
ACP=/nix/store/w2vz79sc7qflws5hs36955svrsxkrdhr-claude-agent-acp-0.88.0/bin/claude-agent-acp
t=$1; Y='---\ndescription: spike yolo agent\nmode: primary\npermission:\n  bash: allow\n  edit: allow\n---\nYou are a test agent.\n'
P2='Sandboxed permission test: nixos-rebuild here is a stub that only logs. Using only your bash tool, run exactly, once: nixos-rebuild switch --dry-run'
mode() { jq -rs '(map(select(.kind=="in" and .msg.result.sessionId))[0].msg.result) | (.modes.currentModeId // ([.configOptions[]? | select(.id=="mode") | .currentValue] | join(",")))' "$1"; }
reqs() { jq -s 'map(select(.kind=="in" and .msg.method=="session/request_permission")) | length' "$1"; }
stat_() { jq -c '[.tool_calls[] | "\(.title): \(.status)"]' "${1%.jsonl}.summary.json"; }
# R1 OpenCode: project default_agent + custom agent, no client choice
D=$SP/$t-oc-default; mkdir -p $D/.opencode/agent; printf '%b' "$Y" >$D/.opencode/agent/yolo.md; echo '{"default_agent":"yolo"}' > $D/opencode.json; rm -f out/nixos-rebuild.calls
printf '%s\n' "Sandboxed permission test. Using only your bash tool, run exactly: touch spike-r1.txt" "$P2" | ./sandbox.sh ./acp_client.py --log out/fix/$t-r1.jsonl --cwd $D -- opencode acp >/dev/null 2>&1
echo "R1 opencode default_agent+custom: start=$(mode out/fix/$t-r1.jsonl) requests=$(reqs out/fix/$t-r1.jsonl) tools=$(stat_ out/fix/$t-r1.jsonl) file=$([[ -e $D/spike-r1.txt ]] && echo CREATED || echo absent) stub=$(cat out/nixos-rebuild.calls 2>/dev/null || echo none)"
# R2 OpenCode: client explicitly selects the custom agent
D=$SP/$t-oc-select; mkdir -p $D/.opencode/agent; printf '%b' "$Y" >$D/.opencode/agent/yolo.md; rm -f out/nixos-rebuild.calls
printf '%s\n' "Sandboxed permission test. Using only your bash tool, run exactly: touch spike-r2.txt" "$P2" | ./sandbox.sh ./acp_client.py --log out/fix/$t-r2.jsonl --cwd $D --config mode=yolo -- opencode acp >/dev/null 2>&1
echo "R2 opencode select custom: offered=$(jq -rs '[map(select(.kind=="in" and .msg.result.sessionId))[0].msg.result.configOptions[] | select(.id=="mode") | .options[].value] | join(",")' out/fix/$t-r2.jsonl) requests=$(reqs out/fix/$t-r2.jsonl) tools=$(stat_ out/fix/$t-r2.jsonl) file=$([[ -e $D/spike-r2.txt ]] && echo CREATED || echo absent) stub=$(cat out/nixos-rebuild.calls 2>/dev/null || echo none)"
# R3 Claude: settings.local.json acceptEdits (logged in)
D=$SP/$t-cl-local; mkdir -p $D/.claude; echo '{"permissions":{"defaultMode":"acceptEdits"}}' > $D/.claude/settings.local.json
echo "Sandboxed permission test. Using only your Bash tool, run exactly this once: touch spike-r3.txt" | CLAUDE_AGENT_ACP_PROTOCOL=v1 ./sandbox.sh ./acp_client.py --log out/fix/$t-r3.jsonl --cwd $D -- $ACP >/dev/null 2>&1
echo "R3 claude local acceptEdits: start=$(mode out/fix/$t-r3.jsonl) requests=$(reqs out/fix/$t-r3.jsonl) tools=$(stat_ out/fix/$t-r3.jsonl) file=$([[ -e $D/spike-r3.txt ]] && echo CREATED || echo absent)"
# R4 Claude: user settings acceptEdits / auto (CLAUDE_CONFIG_DIR, no login: start mode only)
for m in acceptEdits auto; do C=$SP/$t-cl-user-$m; mkdir -p $C $SP/cwd; echo "{\"permissions\":{\"defaultMode\":\"$m\"}}" > $C/settings.json
 : | CLAUDE_CONFIG_DIR=$C CLAUDE_AGENT_ACP_PROTOCOL=v1 ./sandbox.sh ./acp_client.py --log out/fix/$t-r4-$m.jsonl --cwd $SP/cwd --timeout 120 -- $ACP >/dev/null 2>&1
 echo "R4 claude user defaultMode=$m: start=$(mode out/fix/$t-r4-$m.jsonl)"; done
# R5 Claude: client set_mode acceptEdits (client-controlled; recorded, not required to close)
D=$SP/$t-cl-setmode; mkdir -p $D
echo "Sandboxed permission test. Using only your Bash tool, run exactly this once: touch spike-r5.txt" | CLAUDE_AGENT_ACP_PROTOCOL=v1 ./sandbox.sh ./acp_client.py --log out/fix/$t-r5.jsonl --cwd $D --mode acceptEdits -- $ACP >/dev/null 2>&1
echo "R5 claude client set_mode acceptEdits: set_mode=$(jq -rs 'map(select(.kind=="in" and .msg.id==3))[0].msg | (.error.data.details // .error.message // "ok")' out/fix/$t-r5.jsonl) requests=$(reqs out/fix/$t-r5.jsonl) file=$([[ -e $D/spike-r5.txt ]] && echo CREATED || echo absent)"
