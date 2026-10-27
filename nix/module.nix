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
#   - vexos-ai: launcher, first-run picker, Claude accounts, theme sync,
#     "diagnose with AI"
#   - the VexOS skills at /etc/vexos/ai/skills, linked into ~/.claude/skills
#     (Claude Code and OpenCode read it) and ~/.agents/skills (the generic spot)
#   - system-wide (managed) agent policy: never apply (switch/boot/rebuild
#     denied), always ask (no auto-approving modes); see nix/policy.nix
#   - user services: crash watcher, theme watcher, welcome
#   - the assistant panel (vexos-ai-panel, a GTK window over ACP), only with
#     the managed policy above and agents at least as new as the tested ones
#   - on GNOME, the panel's Shell extension (tile and shortcut). Installed
#     only: the module never enables it; each user runs
#     `gnome-extensions enable vexos-assistant@victorytek.github.io` once
{ mkVexosAi, mkVexosAiPanel, mkVexosAiExtension }:
{ config, lib, pkgs, ... }:
let
  cfg = config.programs.vexos-ai;
  package = mkVexosAi pkgs;

  policy = import ./policy.nix { inherit lib; };

  skills = [ "vexos" "vexos-diagnose" ];

  # The panel drives the agents over ACP; these versions were tested live
  # (assistant_panel_spike_results.md). 26.05's claude-agent-acp is 0.36.1.
  minimum = { claudeAcp = "0.88.0"; opencode = "1.19.0"; };

  # The panel runs the pinned agents, whatever else is on the user's PATH.
  panel = pkgs.symlinkJoin {
    name = "vexos-ai-panel";
    paths = [ (mkVexosAiPanel pkgs) ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    # D-Bus activation (the Shell extension) must start the wrapped panel too.
    postBuild = ''
      wrapProgram $out/bin/vexos-ai-panel \
        --prefix PATH : ${lib.makeBinPath [ cfg.claudeAcpPackage cfg.opencodePackage ]}
      service=$out/share/dbus-1/services/io.github.victorytek.VexosAssistant.service
      rm $service
      printf '%s\n' '[D-BUS Service]' 'Name=io.github.victorytek.VexosAssistant' \
        "Exec=$out/bin/vexos-ai-panel --gapplication-service" > $service
    '';
  };

  extension = mkVexosAiExtension pkgs;

  # A managed policy file the panel relies on: installed, enabled, unchanged.
  managed = path: text:
    let etc = config.environment.etc; in
    etc ? ${path} && etc.${path}.enable && etc.${path}.text == text;

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

    enablePanel = lib.mkOption {
      type = lib.types.bool;
      default = cfg.enable;
      defaultText = lib.literalExpression "config.programs.vexos-ai.enable";
      description = ''
        The assistant panel (vexos-ai-panel) and its desktop entry. It needs
        the managed agent policy that `enable` installs, and agents at least
        as new as the ones it was tested with (claude-agent-acp
        ${minimum.claudeAcp}, opencode ${minimum.opencode}).
      '';
    };

    claudeAcpPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.claude-agent-acp;
      defaultText = lib.literalExpression "pkgs.claude-agent-acp";
      description = ''
        The ACP adapter the panel runs Claude Code through. The panel needs
        ${minimum.claudeAcp} or later; nixos-26.05 has 0.36.1, so vexos-nix
        passes the nixpkgs-unstable build.
      '';
    };

    enablePanelExtension = lib.mkOption {
      type = lib.types.bool;
      default = cfg.enablePanel && config.services.desktopManager.gnome.enable;
      defaultText = lib.literalExpression
        "config.programs.vexos-ai.enablePanel && config.services.desktopManager.gnome.enable";
      description = ''
        Install the panel's GNOME Shell extension (${extension.extensionUuid}):
        a Quick Settings tile and a shortcut that show or hide the panel.
        This only installs it; the module never enables extensions. Each
        user enables it once with
        `gnome-extensions enable ${extension.extensionUuid}`.
      '';
    };
  };

  config = lib.mkMerge [ (lib.mkIf cfg.enable {
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
  })

  # ── Panel ──────────────────────────────────────────────────────────────────
  (lib.mkIf cfg.enablePanel {
    assertions = [
      {
        assertion = cfg.enable;
        message = "programs.vexos-ai.enablePanel needs programs.vexos-ai.enable, which installs the managed agent policy.";
      }
      {
        assertion = managed "claude-code/managed-settings.d/50-vexos.json" (builtins.toJSON policy.claude);
        message = "programs.vexos-ai.enablePanel needs Claude's managed policy at /etc/claude-code/managed-settings.d/50-vexos.json, as nix/policy.nix renders it.";
      }
      {
        assertion = managed "opencode/opencode.json" (builtins.toJSON policy.opencode);
        message = "programs.vexos-ai.enablePanel needs OpenCode's managed policy at /etc/opencode/opencode.json, as nix/policy.nix renders it.";
      }
      {
        assertion = lib.versionAtLeast (lib.getVersion cfg.claudeAcpPackage) minimum.claudeAcp;
        message = "programs.vexos-ai.claudeAcpPackage is ${lib.getVersion cfg.claudeAcpPackage}; the panel needs ${minimum.claudeAcp} or later. Pass the nixpkgs-unstable build (pkgs.unstable.claude-agent-acp).";
      }
      {
        assertion = lib.versionAtLeast (lib.getVersion cfg.opencodePackage) minimum.opencode;
        message = "programs.vexos-ai.opencodePackage is ${lib.getVersion cfg.opencodePackage}; the panel needs ${minimum.opencode} or later. Pass the nixpkgs-unstable build (pkgs.unstable.opencode).";
      }
    ];

    environment.systemPackages = [ cfg.claudeAcpPackage panel ];
    services.dbus.packages = [ panel ];
  })

  # ── Panel extension (GNOME Shell) ──────────────────────────────────────────
  (lib.mkIf cfg.enablePanelExtension {
    assertions = [
      {
        assertion = cfg.enablePanel;
        message = "programs.vexos-ai.enablePanelExtension needs programs.vexos-ai.enablePanel: the extension only shows or hides the panel.";
      }
      {
        assertion = extension.metadata.uuid == extension.extensionUuid
          && extension.metadata."shell-version" == [ "50" ];
        message = "programs.vexos-ai: the panel extension's metadata.json must have uuid ${extension.extensionUuid} and shell-version [ \"50\" ].";
      }
    ];

    environment.systemPackages = [ extension ];
  })
  ];
}
