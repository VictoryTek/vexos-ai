#!/usr/bin/env bash
# THROWAWAY SPIKE. Runs a command in a mount namespace where the real managed
# policy files, rendered from nix/module.nix (see README.md), sit at the real
# managed paths. Nothing outside the namespace changes. Also shadows
# nixos-rebuild with a stub that only logs, so a policy failure is harmless.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)

args=(--bind / / --dev /dev --proc /proc --die-with-parent --tmpfs /etc)
for e in /etc/* /etc/.[!.]*; do
  [[ -e $e || -L $e ]] || continue
  if [[ -L $e ]]; then
    args+=(--symlink "$(readlink "$e")" "$e")
  else
    args+=(--bind "$e" "$e")
  fi
done
# SPIKE_NO_MANAGED=1: control run without the policy files.
[[ ${SPIKE_NO_MANAGED-} == 1 ]] || args+=(
  --dir /etc/claude-code/managed-settings.d
  --ro-bind "${SPIKE_CLAUDE_MANAGED:-$here/managed/claude-code_managed-settings.d_50-vexos.json}" /etc/claude-code/managed-settings.d/50-vexos.json
  --dir /etc/opencode
  --ro-bind "${SPIKE_OPENCODE_MANAGED:-$here/managed/opencode_opencode.json}" /etc/opencode/opencode.json
)
# SPIKE_BIND="src:dest": one extra bind inside the namespace (dest must exist).
[[ -n ${SPIKE_BIND-} ]] && args+=(--bind "${SPIKE_BIND%%:*}" "${SPIKE_BIND#*:}")
args+=(
  --ro-bind "$here/stub/nixos-rebuild" "$(readlink -f /run/current-system/sw/bin/nixos-rebuild)"
)
exec bwrap "${args[@]}" -- "$@"
