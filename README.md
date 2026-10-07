# vexos-ai

The VexOS AI assistant: Claude Code or OpenCode, taught how VexOS works
(`skills/`), fenced off from applying changes (managed deny policy), with a
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
- `skills/` — agent skills, linked into `~/.claude/skills`
- `nix/package.nix`, `nix/module.nix`

## Development

`nix develop`, then `nix build` (shellcheck runs as part of the build).
