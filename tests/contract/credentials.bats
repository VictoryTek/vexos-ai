#!/usr/bin/env bats
# Nothing in this repo opens Claude's login file (docs/contract.md
# §Credentials and the network). No script or module names it: not the CLI,
# the Nix files, the crates, the extension, the skills, or the spike scripts.
# Only the docs (docs/ and .github/docs/) and this suite's fixtures may: the
# fixtures plant a fake one under the test HOME to prove that nothing reads or
# leaks it.
#
# VEXOS_AI_SRC is the source tree; the contract check passes the flake's. By
# hand it defaults to the checkout this suite is in.

load helpers
setup() { common_setup; }

@test "no script or module in the repo names .credentials.json" {
  local src=${VEXOS_AI_SRC:-$BATS_TEST_DIRNAME/../..} f hits=()
  # A wrong tree must fail, not pass with nothing scanned.
  [[ -f $src/bin/vexos-ai.sh && -f $src/flake.nix ]] || { echo "not the repo: $src"; return 1; }
  while IFS= read -r -d '' f; do
    # Docs may name it to say it is never read; skills are agent instructions,
    # so they are scanned like code.
    [[ $f == "$src"/docs/*.md || $f == "$src"/.github/docs/*.md ]] && continue
    if grep -qIF '.credentials' "$f"; then hits+=("${f#"$src"/}"); fi
  done < <(find "$src" \( -name .git -o -path "$src/target" -o -path "$src/result*" \
    -o -path "$src/tests/contract" -o -path "$src/spike/acp/out" \) -prune -o -type f -print0)
  [ "${#hits[@]}" -eq 0 ] || { printf 'names the login file: %s\n' "${hits[@]}"; return 1; }
  run grep -cF '.credentials' "$VEXOS_AI_BIN"
  [ "$output" = 0 ]
}
