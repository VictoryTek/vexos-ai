# NixOS module for the VexOS AI assistant: Claude Code and OpenCode, taught how
# VexOS works and fenced off from applying changes. The agent edits /etc/nixos
# side-files and dry-builds; the user runs `just rebuild`.
#
# Install-only: whether and where this runs (role gating, the
# vexos.features.ai toggle) belongs to vexos-nix `modules/ai.nix`, which sets
# programs.vexos-ai.enable — the same split as vex-vpn.
#
# What it installs:
#   - claude-code + opencode (packages are options; vexos-nix passes unstable)
#   - vexos-ai: launcher, first-run picker, Claude accounts and usage
#     warnings, theme sync, "diagnose with AI"
#   - the VexOS skills at /etc/vexos/ai/skills, linked into ~/.claude/skills
#     (Claude Code and OpenCode read it) and ~/.agents/skills (the generic spot)
#   - system-wide (managed) agent policy: never apply (switch/boot/rebuild
#     denied), always ask (no auto-approving modes); see nix/policy.nix
#   - user services: crash watcher, usage timer, theme watcher, welcome
{ mkVexosAi }:
{ config, lib, pkgs, ... }:
let
  cfg = config.programs.vexos-ai;
  package = mkVexosAi pkgs;

  policy = import ./policy.nix { inherit lib; };

  skills = [ "vexos" "vexos-diagnose" ];

  # The system profile on PATH: these services launch claude/opencode, the
  # terminal (xdg-terminal-exec) and vexos-ai itself by name.
  userService = description: script: {
    inherit description;
    partOf   = [ "graphical-session.target" ];
    after    = [ "graphical-session.target" ];
    wantedBy = [ "graphical-session.target" ];
    path     = [ "/run/current-system/sw" ];
    serviceConfig.ExecStart = "${package}/bin/vexos-ai ${script}";
  };
in
{
  options.programs.vexos-ai = {
    enable = lib.mkEnableOption "the VexOS AI assistant (Claude Code or OpenCode)";

    claudePackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.claude-code;
      defaultText = lib.literalExpression "pkgs.claude-code";
      description = ''
        Claude Code package. Unfree: the consumer must allow it. vexos-nix
        passes the nixpkgs-unstable build, as agent CLIs move weekly.
      '';
    };

    opencodePackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.opencode;
      defaultText = lib.literalExpression "pkgs.opencode";
      description = "OpenCode package.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      cfg.claudePackage
      cfg.opencodePackage
      package
    ];

    # ── Skills ───────────────────────────────────────────────────────────────
    environment.etc."vexos/ai/skills".source = ../skills;

    # One link per skill (not the whole directory) so the user's own skills in
    # ~/.claude/skills are left alone. L+ only replaces the link itself.
    # OpenCode reads ~/.claude/skills too; ~/.agents/skills is the generic
    # location other agents use, so the skills are linked there as well.
    systemd.user.tmpfiles.rules = lib.concatMap
      (dir: map (s: "L+ %h/${dir}/${s} - - - - /etc/vexos/ai/skills/${s}") skills)
      [ ".claude/skills" ".agents/skills" ];

    # ── Agent policy ─────────────────────────────────────────────────────────
    # Never apply, always ask: see nix/policy.nix.
    environment.etc."claude-code/managed-settings.d/50-vexos.json".text = builtins.toJSON policy.claude;
    environment.etc."opencode/opencode.json".text = builtins.toJSON policy.opencode;

    # ── User services ────────────────────────────────────────────────────────
    # `vexos-ai crash-capture off` creates the flag file; the unit then does not
    # start (a skipped condition is not a failure). The path is the default
    # ~/.config one: specifiers cannot see an XDG_CONFIG_HOME override.
    systemd.user.services.vexos-ai-crash-watch = lib.recursiveUpdate
      (userService "Offer AI diagnosis when a program crashes" "crash-watch")
      { unitConfig.ConditionPathExists = "!%h/.config/vexos/ai/crash-capture-off"; };
    systemd.user.services.vexos-ai-theme-watch = userService "Keep the AI assistant's theme in sync with the desktop" "theme-watch";
    # Simple, not oneshot: it waits on the notification for as long as the
    # user leaves it, which would trip a oneshot's start timeout.
    systemd.user.services.vexos-ai-welcome = userService "Invite the user to set up the AI assistant" "welcome";

    systemd.user.services.vexos-ai-usage = {
      description = "Check Claude subscription usage and warn near a limit";
      path = [ "/run/current-system/sw" ];
      serviceConfig.Type = "oneshot";
      serviceConfig.ExecStart = "${package}/bin/vexos-ai usage-check";
    };
    # Ticks every 3 minutes, the fastest probe interval (an account near its
    # limit). The script decides per account whether a probe is due: every 10
    # minutes for the active one, 60 for the others, and never during a backoff.
    systemd.user.timers.vexos-ai-usage = {
      description = "Refresh Claude subscription usage when it is due";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnStartupSec = "2min";
        OnUnitActiveSec = "3min";
      };
    };
  };
}
