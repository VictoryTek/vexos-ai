# The assistant's safety policy, as the two managed agent configs that
# nix/module.nix installs. Kept apart from the module so `checks.policy` can
# assert it without evaluating a NixOS system.
#
# Two rules, neither negotiable:
#   1. Agents never apply a configuration: the commands below are denied.
#   2. Agents ask before they act: no auto-approving modes.
#
# Deny rules match the command text the agent writes, so they are a guardrail;
# the real gate is that applying needs root and the agent has no terminal to
# answer sudo with (pkexec shows the user a dialog).
{ lib }:
let
  # Commands that apply a configuration, or evaluate every output at once.
  # Agents must hand these to the user.
  appliers = [
    "nixos-rebuild switch" "nixos-rebuild boot" "nixos-rebuild test"
    "just rebuild" "just switch" "just update" "just update-all"
    "nix flake check"
  ];
  privileged = cmd: [ cmd "sudo ${cmd}" "pkexec ${cmd}" ];
  denied = lib.concatMap privileged appliers;

  # OpenCode allows every tool by default and uses the last matching rule.
  # "*" sorts first in builtins.toJSON, so the denies after it win.
  opencodePermission = {
    edit = "ask";
    bash = { "*" = "ask"; } // lib.genAttrs (map (c: "${c}*") denied) (_: "deny");
  };
in
{
  inherit denied;

  # Claude Code: a managed-settings.d drop-in, so it merges with any other
  # admin policy instead of owning managed-settings.json. Managed values cannot
  # be overridden by user or project settings. Since v2.1.283 an interactive
  # session starts in auto mode, where a classifier approves actions instead of
  # the user; disabling it (and bypass) keeps every session asking.
  claude = {
    permissions = {
      disableAutoMode = "disable";
      disableBypassPermissionsMode = "disable";
      deny = map (c: "Bash(${c} *)") denied;
    };
  };

  # OpenCode: system-wide managed config, loaded after the user's own. An
  # agent's own permission rules are evaluated after the global ones, so the
  # same rules are repeated for each built-in agent that runs tools; otherwise
  # a user's `agent.build.permission.bash."*" = "allow"` would outrank them.
  opencode = {
    "$schema" = "https://opencode.ai/config.json";
    autoupdate = false; # updated by Nix with the rest of the system
    permission = opencodePermission;
    agent = lib.genAttrs [ "build" "general" "plan" "explore" ]
      (_: { permission = opencodePermission; });
  };
}
