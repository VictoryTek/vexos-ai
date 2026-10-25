#!/usr/bin/env bash
# THROWAWAY SPIKE: live smoke runs of crates/vexos-ai-core (examples/smoke.rs)
# against the real agents, inside sandbox.sh. Run from spike/acp.
#   smoke/run.sh NAME AGENT CWD [smoke flags...] < prompts
# Writes out/smoke/NAME.events.jsonl (what the UI saw), NAME/{in,out}.jsonl
# (raw ACP traffic, unless --spawn) and NAME.stub (nixos-rebuild stub calls).
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
name=$1 agent=$2 cwd=$3; shift 3
out=$here/out/smoke; mkdir -p "$out/$name" "$cwd"
rm -f "$here/out/nixos-rebuild.calls"
bin=$here/../../target/debug/examples/smoke
acp=/nix/store/w2vz79sc7qflws5hs36955svrsxkrdhr-claude-agent-acp-0.88.0/bin
tee_args=(--tee "$out/$name")
for a in "$@"; do [[ $a == --spawn ]] && tee_args=(); done
PATH=$acp:$PATH timeout 600 "$here/sandbox.sh" "$bin" --agent "$agent" --cwd "$cwd" "${tee_args[@]}" "$@" \
  > "$out/$name.events.jsonl" 2> "$out/$name.stderr"
echo "exit=$?" >> "$out/$name.events.jsonl"
cp "$here/out/nixos-rebuild.calls" "$out/$name.stub" 2>/dev/null || echo none > "$out/$name.stub"
cat "$out/$name.events.jsonl"
echo "stub: $(cat "$out/$name.stub")"
