# vexos-ai

The VexOS AI assistant: Claude Code (or Codex, or OpenCode: `vexos-ai pick`), taught how VexOS works
(`skills/`), fenced off from applying changes and kept asking before it acts
(managed policy, `nix/policy.nix`), with a
launcher, theme sync and "diagnose with AI".

Consumed by [vexos-nix](https://github.com/VictoryTek/vexos-nix) as a flake
input. Users enable it there with `just feature enable ai`; this repo is
install-only (`programs.vexos-ai.enable`).

## Outputs

| Output | |
|---|---|
| `packages.<system>.default` | the `vexos-ai` command + desktop entry |
| `packages.<system>.vexos-ai-panel` | the assistant window (GTK) |
| `packages.<system>.vexos-ai-panel-extension` | the GNOME Shell extension: top-bar icon and quick panel, Quick Settings tile, shortcut |
| `packages.<system>.vexos-ai-usage` | the usage service behind the quick panel's plan limits |
| `nixosModules.default` | `programs.vexos-ai.{enable,claudePackage,enablePanel,enablePanelExtension,enableUsage,...}` |
| `overlays.default` | `pkgs.vexos-ai` |

## Layout

- `bin/vexos-ai.sh` — launcher and helpers
- `crates/` — the assistant window, its core and view-model, and the usage service
- `extension/` — the GNOME Shell extension (top-bar panel with starters and plan limits)
- `skills/` — agent skills, linked into `~/.claude/skills` and `~/.agents/skills`
- `nix/package.nix`, `nix/module.nix`
- `nix/policy.nix` — the managed agent policy (asserted by `checks.policy`)
- `docs/contract.md` — argv, exit codes and `status --json` for callers (VexPortal)
- `tests/contract/` — the black-box bats suite for that contract (`checks.contract`)
- `docs/spec.md` — roadmap and decisions

## The top-bar panel

On GNOME the extension adds an icon to the top bar. Left-click opens a quick
panel: Claude Code's plan limits (Session and Weekly, with reset times) and
three starter tasks. Right-click shows or hides the chat window, middle-click
refreshes the limits, and the icon turns red at 90% of a limit. A starter only
puts a draft in the chat's input; nothing is sent until you click Send.

Plan limits are **off until you switch them on** in the panel. Switched on, a
small service reads Claude Code's saved sign-in and asks Anthropic's usage
endpoint for your limits (the same one Claude Code's `/usage` uses). It is the
only code in this repo that opens the sign-in; see "Usage service" in
`docs/contract.md`.

Closing the chat window hides it and keeps the session warm, so it reopens at
once; a window hidden for 10 minutes ends its session.

## Safety limits

The deny rules match the command text an agent writes, so they are a guardrail:
the real gate is that applying needs root, and `sudo`/`pkexec` ask the user.
Claude Code applies only its highest-ranked managed source. If you sign in with
a work organization that pushes server-managed settings, the VexOS file policy
is skipped; check `/status` → "Setting sources" inside Claude Code.

## Development

`nix develop`, then `nix build` (shellcheck runs as part of the build) and
`nix flake check` (adds the policy and contract checks).
