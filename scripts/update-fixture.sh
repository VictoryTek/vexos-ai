#!/usr/bin/env bash
# Regenerates tests/fixtures/vexos-nix.json from a vexos-nix checkout: the
# public `just` recipes (with their parameters) and the /etc/nixos side-files.
# checks.skills compares what skills/ name against this file. Run it when
# vexos-nix's justfile or template/etc-nixos-flake.nix changes:
#
#   scripts/update-fixture.sh ../vexos-nix
#
# Needs `just`, `jq` and `git`. Writes only the fixture.
set -euo pipefail

src=${1:?usage: update-fixture.sh <vexos-nix checkout>}
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out=$repo/tests/fixtures/vexos-nix.json
template=$src/template/etc-nixos-flake.nix

[ -f "$src/justfile" ] || { echo "no justfile in $src" >&2; exit 1; }
[ -f "$template" ] || { echo "no $template" >&2; exit 1; }

rev=$(git -C "$src" rev-parse HEAD)

# Every file the host flake wires in: `./name.nix`.
wired=$(grep -oE '\./[A-Za-z0-9_-]+\.nix' "$template" | sed 's#^\./##' | sort -u)
[ -n "$wired" ] || { echo "no side-files found in $template" >&2; exit 1; }

# Names the skills use that the template does not wire in as a module: the
# host flake itself, the variant file the installer writes, and the secrets
# directory.
extra=$'flake.nix\nvexos-variant\nsecrets/'

recipes=$(just --justfile "$src/justfile" --working-directory "$src" \
  --dump --dump-format json |
  jq -S '.recipes
    | to_entries
    | map(select(.value.private | not))
    | map({key: .key, value: [.value.parameters[].name]})
    | from_entries')

mkdir -p "$(dirname "$out")"
printf '%s\n%s\n' "$wired" "$extra" | sort -u | jq -R . | jq -s . |
  jq -S --arg rev "$rev" --argjson recipes "$recipes" \
    '{rev: $rev, recipes: $recipes, sideFiles: .}' > "$out"

echo "wrote $out (vexos-nix $rev)"
