# vexos-ai

Install-only flake for the VexOS AI assistant, consumed by vexos-nix.

- The feature toggle (`vexos.features.ai.enable`), role gating and the
  `pkgs.unstable` package choice live in vexos-nix `modules/ai.nix`, not here.
- `skills/` describe vexos-nix's justfile and `/etc/nixos` side-files. When
  those change in vexos-nix, update the skills here and bump the lock there.
- `nix/policy.nix` is the assistant's safety policy, asserted by
  `checks.policy`: agents must never be able to apply a configuration
  (`nixos-rebuild switch|boot|test`, `just rebuild|switch|update|update-all`)
  or run `nix flake check`, and must always ask before acting (no Claude auto
  or bypass mode; OpenCode `ask` for edit and bash).
- Never run `nixos-rebuild switch|boot` from automation; validate with
  `nix build` here and `dry-build` in vexos-nix.
