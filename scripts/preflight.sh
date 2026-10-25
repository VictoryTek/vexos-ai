#!/usr/bin/env bash
# Preflight: what CI checks, one check at a time (never `nix flake check`,
# which builds every check for every system). Needs Nix with flakes, on Linux
# or WSL. Run from the repository root:
#
#   scripts/preflight.sh            # the flake as git sees it (tracked files)
#   scripts/preflight.sh path:DIR   # a directory as is, untracked files too
set -euo pipefail

flake=${1:-.}
system=$(nix eval --raw --impure --expr builtins.currentSystem)

step() { printf '\n== %s\n' "$*"; }

step "package (shellcheck runs here)"
nix build --no-link "$flake"

for check in policy contract core panel; do
  step "checks.$system.$check"
  nix build --no-link -L "$flake#checks.$system.$check"
done

step "NixOS module evaluates"
nix eval "$flake#nixosModules.default" --apply 'm: builtins.typeOf m'

step "rustfmt"
nix develop "$flake" -c cargo fmt --all --check

printf '\npreflight passed\n'
