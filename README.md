# vexos-ai

The VexOS AI assistant: Claude Code or OpenCode, taught how VexOS works
(`skills/`), fenced off from applying changes and kept asking before it acts
(managed policy, `nix/policy.nix`), with a
launcher, Claude account/usage handling, theme sync and "diagnose with AI".

Consumed by [vexos-nix](https://github.com/VictoryTek/vexos-nix) as a flake
input. Users enable it there with `just feature enable ai`; this repo is
install-only (`programs.vexos-ai.enable`).

## Outputs

| Output | |
|---|---|
| `packages.<system>.default` | the `vexos-ai` command + desktop entry |
| `nixosModules.default` | `programs.vexos-ai.{enable,claudePackage,opencodePackage}` |
| `overlays.default` | `pkgs.vexos-ai` |

## Layout

- `bin/vexos-ai.sh` — launcher and helpers
- `skills/` — agent skills, linked into `~/.claude/skills` and `~/.agents/skills`
- `nix/package.nix`, `nix/module.nix`
- `nix/policy.nix` — the managed agent policy (asserted by `checks.policy`)
- `docs/contract.md` — argv, exit codes and `status --json` for callers (VexPortal)
- `tests/contract/` — the black-box bats suite for that contract (`checks.contract`)
- `docs/spec.md` — roadmap and decisions

## Safety limits

The deny rules match the command text an agent writes, so they are a guardrail:
the real gate is that applying needs root, and `sudo`/`pkexec` ask the user.
Claude Code applies only its highest-ranked managed source. If you sign in with
a work organization that pushes server-managed settings, the VexOS file policy
is skipped; check `/status` → "Setting sources" inside Claude Code.

## Development

`nix develop`, then `nix build` (shellcheck runs as part of the build) and
`nix flake check` (adds the policy and contract checks).
