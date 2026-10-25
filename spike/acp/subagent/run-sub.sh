#!/usr/bin/env bash
# THROWAWAY SPIKE. run-sub.sh NAME SCOPE(project|user) [extra env...]: the Claude
# subagent route, every request denied. "user" puts the agent in a temporary
# CLAUDE_CONFIG_DIR and binds only the real credentials file into it.
# shellcheck disable=SC2086
set -u
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/.." || exit 1
S=${SPIKE_SCRATCH:-/tmp/vexos-ai-spike-subagent}   # scratch dirs, outside any repo
mkdir -p "$S"; cp "$here/yolo.md" "$S/"
n=$1 scope=$2; shift 2; D=$S/$n; mkdir -p $D; env=("$@")
if [[ $scope == project ]]; then mkdir -p $D/.claude/agents; cp $S/yolo.md $D/.claude/agents/
else C=$S/$n-config; mkdir -p $C/agents; cp $S/yolo.md $C/agents/; : > $C/.credentials.json
  env+=("CLAUDE_CONFIG_DIR=$C" "SPIKE_BIND=$HOME/.claude/.credentials.json:$C/.credentials.json"); fi
printf 'hello\n' > $D/existing.txt
echo 'Sandboxed permission test. Use your Agent tool to delegate to the yolo subagent with exactly this instruction: "Do these three steps, one tool call each, even if one is refused: (1) with your Bash tool run exactly: touch sub-bash.txt (2) with your Write tool create sub-write.txt containing hi (3) read existing.txt with your Read tool, then with your Edit tool replace hello with bye in it." Do not do any of it yourself.' \
  | env "${env[@]}" smoke/run.sh $n claude $D >/dev/null
echo "  agents offered: $(jq -rs '[.[] | select(.id==2) | [.result.configOptions[]? | select(.id=="agent") | .options[].value]] | first | join(",")' out/smoke/$n/out.jsonl)"
echo "  permission requests: $(jq -c 'select(.method=="session/request_permission") | .params.toolCall.title' out/smoke/$n/out.jsonl | tr '\n' ' ')"
echo "  results: $(jq -c 'select(.params.update.sessionUpdate=="tool_call_update" and (.params.update.status=="completed" or .params.update.status=="failed")) | "\(.params.update.status): \((.params.update.content // .params.update.rawOutput // "" | tostring)[30:100])"' out/smoke/$n/out.jsonl | head -5 | tr '\n' ' ')"
echo "  files: [$(find $D -mindepth 1 -maxdepth 1 ! -name .claude -printf '%f ')] existing.txt=$(cat $D/existing.txt) stub=$(cat out/smoke/$n.stub)"
