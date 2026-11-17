# NixOS module for the VexOS AI assistant: Claude Code, Codex or OpenCode, taught how
# VexOS works and fenced off from applying changes. The agent edits /etc/nixos
# side-files and dry-builds; the user runs `just rebuild`.
#
# Install-only: whether and where this runs (role gating, the
# vexos.features.ai toggle) belongs to vexos-nix `modules/ai.nix`, which sets
# programs.vexos-ai.enable — the same split as vex-vpn.
#
# What it installs:
#   - claude-code (the package is an option; vexos-nix passes unstable). Codex
#     and OpenCode are optional: their packages are null unless a consumer sets
#     them (vexos-nix installs the CLIs itself), but their managed policy is
#     always installed, so choosing one with `vexos-ai pick` is as safe as Claude
#   - vexos-ai: launcher, first-run picker, theme sync, "diagnose with AI"
#   - nothing for the skills: they are shipped in skills/ and exposed as the
#     read-only option programs.vexos-ai.skills ({ path; names; }). The consumer
#     (vexos-nix) links them into ~/.claude/skills and ~/.agents/skills
#   - system-wide (managed) agent policy: never apply (switch/boot/rebuild
#     denied), always ask (no auto-approving modes); see nix/policy.nix
#   - user services: crash watcher, theme watcher, welcome
#   - the assistant panel (vexos-ai-panel, a GTK window over ACP), only with
#     the managed policy above and agents at least as new as the tested ones
#   - on GNOME, the panel's Shell extension (tile and shortcut). Installed
#     only: the module never enables it; each user runs
#     `gnome-extensions enable vexos-assistant@victorytek.github.io` once
#   - with the extension, the usage service (vexos-ai-usage): D-Bus activated,
#     it reads Claude's plan limits for the extension's top-bar panel, and
#     reads nothing until the user switches plan limits on in that panel
{ mkVexosAi, mkVexosAiPanel, mkVexosAiExtension, mkVexosAiUsage }:
{ config, lib, pkgs, ... }:
let
  cfg = config.programs.vexos-ai;
  package = mkVexosAi pkgs;

  policy = import ./policy.nix { inherit lib; };

  # The skills this repo ships. Read from the source path, never from a built
  # derivation, so reading `names` builds nothing.
  skillsDir = ../skills;
  skillNames = lib.attrNames
    (lib.filterAttrs (_: type: type == "directory") (builtins.readDir skillsDir));

  # The panel drives the agents over ACP; the Claude version was tested live
  # (assistant_panel_spike_results.md). 26.05's claude-agent-acp is 0.36.1. The
  # Codex and OpenCode floors only keep out 26.05's builds (codex-acp 0.13.0,
  # opencode 1.15.10), whose modes and permission config differ; neither has
  # been run live here yet.
  minimum = { claudeAcp = "0.88.0"; codexAcp = "2.0.0"; opencode = "2.0.0"; };

  # The optional agent packages that are set.
  optionalAgents = lib.filter (p: p != null) [ cfg.codexPackage cfg.codexAcpPackage cfg.opencodePackage ];

  # The Codex managed requirements, as the TOML Codex reads.
  codexRequirements = (pkgs.formats.toml { }).generate "requirements.toml" policy.codex;

  # The panel runs the pinned agents, whatever else is on the user's PATH.
  panel = pkgs.symlinkJoin {
    name = "vexos-ai-panel";
    paths = [ (mkVexosAiPanel pkgs) ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    # D-Bus activation (the Shell extension) must start the wrapped panel too.
    postBuild = ''
      wrapProgram $out/bin/vexos-ai-panel \
        --prefix PATH : ${lib.makeBinPath ([ cfg.claudeAcpPackage ]
          ++ lib.filter (p: p != null) [ cfg.codexAcpPackage cfg.opencodePackage ])}
      service=$out/share/dbus-1/services/io.github.victorytek.VexosAssistant.service
      rm $service
      printf '%s\n' '[D-BUS Service]' 'Name=io.github.victorytek.VexosAssistant' \
        "Exec=$out/bin/vexos-ai-panel --gapplication-service" > $service
    '';
  };

  extension = mkVexosAiExtension pkgs;
  # The usage service asks `codex app-server` for Codex's limits, so the Codex
  # the module was given is on its PATH; with none, it finds `codex` on the
  # user's PATH, or shows no Codex section.
  usage =
    if cfg.codexPackage == null then mkVexosAiUsage pkgs
    else pkgs.symlinkJoin {
      name = "vexos-ai-usage";
      paths = [ (mkVexosAiUsage pkgs) ];
      nativeBuildInputs = [ pkgs.makeWrapper ];
      # D-Bus activation must start the wrapped service too.
      postBuild = ''
        wrapProgram $out/bin/vexos-ai-usage \
          --prefix PATH : ${lib.makeBinPath [ cfg.codexPackage ]}
        service=$out/share/dbus-1/services/io.github.victorytek.VexosAssistant.Usage.service
        rm $service
        printf '%s\n' '[D-BUS Service]' 'Name=io.github.victorytek.VexosAssistant.Usage' \
          "Exec=$out/bin/vexos-ai-usage --gapplication-service" > $service
      '';
    };

  # A managed policy file the panel relies on: installed, enabled, unchanged.
  managed = path: text:
    let etc = config.environment.etc; in
    etc ? ${path} && etc.${path}.enable && etc.${path}.text == text;

  # The same for a file installed from a store path.
  managedSource = path: source:
    let etc = config.environment.etc; in
    etc ? ${path} && etc.${path}.enable && etc.${path}.source == source;

  # The system profile on PATH: these services launch claude, the
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
    enable = lib.mkEnableOption "the VexOS AI assistant (Claude Code, Codex or OpenCode)";

    claudePackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.claude-code;
      defaultText = lib.literalExpression "pkgs.claude-code";
      description = ''
        Claude Code package. Unfree: the consumer must allow it. vexos-nix
        passes the nixpkgs-unstable build, as agent CLIs move weekly.
      '';
    };

    codexPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      description = ''
        Codex CLI. Null (the default) installs nothing: the consumer installs it
        itself, as vexos-nix does. Set it to have this module install it. Unfree.
      '';
    };

    codexAcpPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      description = ''
        The ACP adapter the panel runs Codex through (`codex-acp`). Null leaves
        Codex out of the panel: `codex-acp` is not on its PATH, so a session
        with Codex chosen fails to start. Needs ${minimum.codexAcp} or later;
        nixos-26.05 has 0.13.0, so pass the nixpkgs-unstable build.
      '';
    };

    opencodePackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      description = ''
        OpenCode, which is its own ACP adapter (`opencode acp`). Null installs
        nothing and leaves the panel to find `opencode` on the user's PATH. Needs
        ${minimum.opencode} or later; nixos-26.05 has 1.15.10, so pass the
        nixpkgs-unstable build.
      '';
    };

    skills = lib.mkOption {
      type = lib.types.attrs;
      readOnly = true;
      description = ''
        The agent skills this repo ships, for the consumer to link: `path` is the
        directory that holds them, `names` the skill directories under it. Read
        only, and set whether or not `enable` is. This module links nothing:
        vexos-nix owns the central skills location and the links into
        `~/.claude/skills` and `~/.agents/skills`. Used on its own, link
        `skills.path` yourself.
      '';
    };

    enablePanel = lib.mkOption {
      type = lib.types.bool;
      default = cfg.enable;
      defaultText = lib.literalExpression "config.programs.vexos-ai.enable";
      description = ''
        The assistant panel (vexos-ai-panel) and its desktop entry. It needs
        the managed agent policy that `enable` installs, and an agent adapter at least
        as new as the one it was tested with (claude-agent-acp
        ${minimum.claudeAcp}).
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

    enableUsage = lib.mkOption {
      type = lib.types.bool;
      default = cfg.enablePanelExtension;
      defaultText = lib.literalExpression "config.programs.vexos-ai.enablePanelExtension";
      description = ''
        Install the usage service (vexos-ai-usage), which the extension's
        top-bar panel asks for plan limits: Claude's, from Claude Code's saved
        sign-in, and Codex's, from `codex app-server` (Codex's own login; this
        service never reads it). Installing it reads nothing: the service stays
        idle, and asks only after the user turns "Show plan limits" on in the
        panel. With `codexPackage` set, that Codex is the one it asks.
      '';
    };
  };

  config = lib.mkMerge [
  { programs.vexos-ai.skills = { path = skillsDir; names = skillNames; }; }

  (lib.mkIf cfg.enable {
    environment.systemPackages = [
      cfg.claudePackage
      package
    ] ++ optionalAgents;

    # ── Agent policy ─────────────────────────────────────────────────────────
    # Never apply, always ask: see nix/policy.nix.
    environment.etc."claude-code/managed-settings.d/50-vexos.json".text = builtins.toJSON policy.claude;
    # Codex reads /etc/codex/requirements.toml as an admin-enforced layer that
    # user and project config cannot override; OpenCode loads
    # /etc/opencode/opencode.json after the user's config. Installed whether or
    # not the agents' packages are, so they are fenced the moment they exist.
    environment.etc."codex/requirements.toml".source = codexRequirements;
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
        assertion = managedSource "codex/requirements.toml" codexRequirements;
        message = "programs.vexos-ai.enablePanel needs Codex's managed policy at /etc/codex/requirements.toml, as nix/policy.nix renders it.";
      }
      {
        assertion = managed "opencode/opencode.json" (builtins.toJSON policy.opencode);
        message = "programs.vexos-ai.enablePanel needs OpenCode's managed policy at /etc/opencode/opencode.json, as nix/policy.nix renders it.";
      }
      {
        assertion = cfg.codexAcpPackage == null
          || lib.versionAtLeast (lib.getVersion cfg.codexAcpPackage) minimum.codexAcp;
        message = "programs.vexos-ai.codexAcpPackage is ${lib.getVersion cfg.codexAcpPackage}; the panel needs ${minimum.codexAcp} or later. Pass the nixpkgs-unstable build (pkgs.unstable.codex-acp).";
      }
      {
        assertion = cfg.opencodePackage == null
          || lib.versionAtLeast (lib.getVersion cfg.opencodePackage) minimum.opencode;
        message = "programs.vexos-ai.opencodePackage is ${lib.getVersion cfg.opencodePackage}; the panel needs ${minimum.opencode} or later. Pass the nixpkgs-unstable build (pkgs.unstable.opencode).";
      }
      {
        assertion = lib.versionAtLeast (lib.getVersion cfg.claudeAcpPackage) minimum.claudeAcp;
        message = "programs.vexos-ai.claudeAcpPackage is ${lib.getVersion cfg.claudeAcpPackage}; the panel needs ${minimum.claudeAcp} or later. Pass the nixpkgs-unstable build (pkgs.unstable.claude-agent-acp).";
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

  # ── Usage service ──────────────────────────────────────────────────────────
  (lib.mkIf cfg.enableUsage {
    assertions = [
      {
        assertion = cfg.enablePanelExtension;
        message = "programs.vexos-ai.enableUsage needs programs.vexos-ai.enablePanelExtension: the service only answers the extension's top-bar panel.";
      }
    ];

    environment.systemPackages = [ usage ];
    services.dbus.packages = [ usage ];
  })
  ];
}
