#!/usr/bin/env bash
# Checks a host after `just rebuild` (and a reboot or re-login, for PATH)
# that the assistant panel is installed the way nix/module.nix says.
# Read-only: it reads files and evaluates nix/policy.nix, and never starts the
# panel or an agent. (Nix may update its own cache in ~/.cache/nix while
# evaluating.) Run it from a checkout of the same vexos-ai commit vexos-nix
# pins, so the expected policy matches what was installed:
#
#   scripts/verify-host.sh
#
# Env: VEXOS_NIXPKGS, a nixpkgs path for `lib` (default: <nixpkgs> from
# NIX_PATH, which flake-based NixOS sets to the system's nixpkgs).
set -uo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
apps=/run/current-system/sw/share/applications
desktop=io.github.victorytek.VexosAssistant.desktop
# The versions the panel was tested with; nix/module.nix asserts the same.
min_acp=0.88.0
min_opencode=1.19.0

failures=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s: %s\n' "$1" "$2"; failures=$((failures + 1)); }

# version_at_least HAVE WANT
version_at_least() {
  [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}

# The version in a store path's name: /nix/store/<hash>-<name>-<version>/…
store_version() {
  local name=$1 path=$2
  sed -n "s#^/nix/store/[a-z0-9]*-${name}-\([0-9][^/]*\)/.*#\1#p" <<<"$path"
}

# ── Managed policy ────────────────────────────────────────────────────────────
# expected_policy claude|opencode: the JSON nix/policy.nix renders.
expected_policy() {
  local lib
  if [ -n "${VEXOS_NIXPKGS-}" ]; then lib="import ${VEXOS_NIXPKGS}/lib"; else lib="import <nixpkgs/lib>"; fi
  nix-instantiate --eval --strict --json \
    --expr "(import ${repo}/nix/policy.nix { lib = ${lib}; }).$1" 2>/dev/null
}

check_policy() {
  local agent=$1 rel=$2 path=/etc/$2 item="managed policy $2"
  local resolved expected
  if [ ! -f "$path" ]; then
    fail "$item" "not installed"
    return
  fi
  resolved=$(readlink -f "$path")
  if [ "$resolved" != "$(readlink -f "/etc/static/$rel")" ] || [[ $resolved != /nix/store/* ]]; then
    fail "$item" "not the file the running system installed ($resolved)"
    return
  fi
  if ! command -v nix-instantiate >/dev/null || ! command -v jq >/dev/null; then
    fail "$item" "needs nix-instantiate and jq to compare it"
    return
  fi
  if ! expected=$(expected_policy "$agent"); then
    fail "$item" "could not render nix/policy.nix (set VEXOS_NIXPKGS?)"
    return
  fi
  if [ "$(jq -S . "$path")" = "$(jq -S . <<<"$expected")" ]; then
    pass "$item matches nix/policy.nix"
  else
    fail "$item" "differs from what nix/policy.nix renders"
  fi
}

check_policy claude claude-code/managed-settings.d/50-vexos.json
check_policy opencode opencode/opencode.json

# ── Programs on PATH (located, never run) ────────────────────────────────────
# check_program NAME [MIN]: on PATH, and at least MIN by its store path.
check_program() {
  local name=$1 min=${2-} found resolved version
  if ! found=$(command -v "$name"); then
    fail "$name on PATH" "not found"
    return
  fi
  resolved=$(readlink -f "$found")
  if [ -z "$min" ]; then
    pass "$name on PATH ($resolved)"
    return
  fi
  version=$(store_version "$name" "$resolved")
  if [ -z "$version" ]; then
    fail "$name on PATH" "cannot read its version from $resolved"
  elif version_at_least "$version" "$min"; then
    pass "$name $version on PATH (at least $min)"
  else
    fail "$name on PATH" "$version is older than $min"
  fi
}

check_program vexos-ai-panel
check_program claude-agent-acp "$min_acp"
check_program opencode "$min_opencode"

# The panel puts the pinned agents first on its own PATH (nix/module.nix).
if panel=$(command -v vexos-ai-panel) && acp=$(command -v claude-agent-acp); then
  acp_bin=$(dirname "$(readlink -f "$acp")")
  if grep -qF "$acp_bin" "$(readlink -f "$panel")"; then
    pass "the panel runs this claude-agent-acp ($acp_bin)"
  else
    fail "the panel runs this claude-agent-acp" "its wrapper does not name $acp_bin"
  fi
fi

# ── Desktop entry ─────────────────────────────────────────────────────────────
if [ ! -f "$apps/$desktop" ]; then
  fail "desktop entry" "$apps/$desktop not found"
elif grep -q '^Exec=vexos-ai-panel' "$apps/$desktop"; then
  pass "desktop entry $desktop"
else
  fail "desktop entry" "$desktop does not run vexos-ai-panel"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "all checks passed"
else
  echo "$failures check(s) failed"
  exit 1
fi
