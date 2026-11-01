#!/usr/bin/env bash
# Stop hook: verifies the working tree when Claude finishes, once per tree state.
#
# - Runs scripts/preflight.sh on the git tree. A pass is remembered, so the same
#   tree is never verified twice.
# - A failure is reported once (exit 2, stderr goes to Claude), briefly, and is
#   remembered: the same tree never blocks a second time, so Claude's reply to the
#   report does not trigger another one. That memory does not depend on
#   `stop_hook_active`, which Claude Code documents but which did not stop the
#   repeats in this harness. `stop_hook_active` is honoured too.
# - If preflight fails only because new files are untracked (git cannot see them,
#   nix builds the git tree), a second preflight runs on a `path:` copy of the
#   tree under target/. If that passes, the one-line message below is the only
#   output. No git write command is ever run.
#
# State (ignored by git): target/.verify-state. hook.log records, per run, what the
# stop_hook_active field was, the tree signature and the decision. No text of the
# conversation is logged.
input="$(cat)"

cd "${CLAUDE_PROJECT_DIR:-.}" 2>/dev/null || exit 0
root="$(pwd)"
state="$root/target/.verify-state"
mkdir -p "$state" 2>/dev/null || exit 0
log() { printf '%s %s\n' "$(date -Is)" "$*" >>"$state/hook.log" 2>/dev/null; }

active=absent
if printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
  active=true
elif printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*false'; then
  active=false
fi

# Claude was sent back by this hook already: let it stop and report.
if [ "$active" = true ]; then
  log "active=true -> allow"
  exit 0
fi

# Nothing changed in the working tree (or not a git repo): nothing to verify.
[ -z "$(git status --porcelain 2>/dev/null)" ] && exit 0

# The verify command cannot run on this machine: skip with a note instead of failing every turn.
if ! { command -v nix; } >/dev/null 2>&1; then
  echo "skipping verify: Nix not found; this project builds on Linux or WSL only" >&2
  exit 0
fi

# What the tree is: status, every tracked change (staged or not) and the contents of
# untracked files. A new state gets a new verification.
sig="$(
  {
    git status --porcelain
    git diff HEAD
    git ls-files -o --exclude-standard -z | xargs -0 -r sha256sum
  } 2>/dev/null | sha256sum | cut -c1-16
)"

if [ -e "$state/$sig.pass" ]; then
  log "active=$active sig=$sig already passed -> allow"
  exit 0
fi
if [ -e "$state/$sig.fail" ]; then
  log "active=$active sig=$sig already reported -> allow"
  exit 0
fi

# Group the command so output from every part of a chained command is captured.
output="$( { scripts/preflight.sh; } 2>&1 )"
status=$?

if [ "$status" -eq 0 ]; then
  : >"$state/$sig.pass"
  log "active=$active sig=$sig preflight passed -> allow"
  exit 0
fi

# Failed. Only because new files are untracked? Build a path: copy of the tree
# (tracked and untracked files) in a fresh directory and run the same preflight.
untracked="$(git ls-files -o --exclude-standard 2>/dev/null)"
# The hook is cut off at 600 s (settings.json): the copy run gets what is left.
left=$((570 - SECONDS))
if [ -n "$untracked" ] && [ "$left" -ge 60 ]; then
  copy="$root/target/verify-copy-$sig"
  if mkdir "$copy" 2>/dev/null &&
    git ls-files -co --exclude-standard -z | tar --null -T - -cf - | tar -xf - -C "$copy"; then
    if command -v timeout >/dev/null 2>&1; then
      run=(timeout "$left" scripts/preflight.sh "path:$copy")
    else
      run=(scripts/preflight.sh "path:$copy")
    fi
    if "${run[@]}" >"$state/$sig.copy.log" 2>&1; then
      : >"$state/$sig.fail"
      log "active=$active sig=$sig failed (exit $status) only because of untracked files -> stage message"
      echo "Stage the new files (VS Code Source Control, click +), then say continue." >&2
      exit 2
    fi
  fi
fi

: >"$state/$sig.fail"
log "active=$active sig=$sig failed (exit $status) -> report"
{
  echo "Verification failed (exit $status). Fix the cause; do not repeat the final report. Last lines:"
  echo "$output" | tail -n 15
} >&2
exit 2
