#!/usr/bin/env bash
# Fails when a skill names a vexos-nix `just` recipe or an /etc/nixos side-file
# that tests/fixtures/vexos-nix.json (scripts/update-fixture.sh) does not have.
#
#   scripts/check-skill-drift.sh <skills-dir> <fixture.json>
#
# What counts as a reference:
#   - `just <recipe>` inside a backtick span or a fenced block (so prose such as
#     "just frame 0" is not one); options like `just --list` are not recipes;
#   - /etc/nixos/<name> anywhere, and a backticked bare `<name>.nix`.
# Names with a placeholder (`configuration-<role>.nix`, also after /etc/nixos/)
# are not matched. A fence may be indented (a list item); tilde fences are not
# recognized (the skills use backtick fences).
set -euo pipefail

skills=${1:?usage: check-skill-drift.sh <skills-dir> <fixture.json>}
fixture=${2:?usage: check-skill-drift.sh <skills-dir> <fixture.json>}

# One line per reference: file:line:kind:name
refs() {
  awk '
    function emit(kind, name) { printf "%s:%d:%s:%s\n", FILENAME, FNR, kind, name }
    FNR == 1 { fence = 0 }
    /^ *```/ { fence = !fence; next }
    {
      line = $0
      # What may hold a recipe: a fenced line without its trailing comment, or
      # the backtick spans of a prose line.
      if (fence) {
        text = line
        sub(/[ \t]+#.*$/, "", text)
      } else {
        text = ""
        rest = line
        while (match(rest, /`[^`]*`/)) {
          text = text " " substr(rest, RSTART + 1, RLENGTH - 2)
          rest = substr(rest, RSTART + RLENGTH)
        }
      }
      t = text
      while (match(t, /(^|[^A-Za-z0-9_-])just +[a-z][a-z0-9_-]*/)) {
        hit = substr(t, RSTART, RLENGTH)
        t = substr(t, RSTART + RLENGTH)
        sub(/^.*just +/, "", hit)
        emit("recipe", hit)
      }
      t = line
      while (match(t, /\/etc\/nixos\/[A-Za-z0-9._-]+\/?/)) {
        hit = substr(t, RSTART, RLENGTH)
        placeholder = substr(t, RSTART + RLENGTH, 1) == "<"
        t = substr(t, RSTART + RLENGTH)
        sub(/^\/etc\/nixos\//, "", hit)
        sub(/[.,]+$/, "", hit)
        if (hit != "" && !placeholder) emit("sidefile", hit)
      }
      if (!fence) {
        t = line
        while (match(t, /`[A-Za-z0-9_-]+\.nix`/)) {
          hit = substr(t, RSTART + 1, RLENGTH - 2)
          t = substr(t, RSTART + RLENGTH)
          emit("sidefile", hit)
        }
      }
    }
  ' "$skills"/*/SKILL.md
}

rev=$(jq -r '.rev' "$fixture")
bad=0
count=0
while IFS=: read -r file line kind name; do
  count=$((count + 1))
  case $kind in
    recipe)
      jq -e --arg n "$name" '.recipes | has($n)' "$fixture" >/dev/null || {
        echo "$file:$line: \`just $name\` is not a recipe in vexos-nix $rev" >&2
        bad=1
      } ;;
    sidefile)
      jq -e --arg n "$name" '.sideFiles | index($n) != null' "$fixture" >/dev/null || {
        echo "$file:$line: side-file \`$name\` is not in vexos-nix $rev" >&2
        bad=1
      } ;;
  esac
done < <(refs)

[ "$count" -gt 0 ] || { echo "no references found in $skills: the scan is broken" >&2; exit 1; }
[ "$bad" = 0 ] || exit 1
echo "skills match vexos-nix $rev ($count references)"
