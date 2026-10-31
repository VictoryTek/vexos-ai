# The assistant's safety policy, as the managed Claude Code config that
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

  # What Claude's acceptEdits mode approves without asking: every file write,
  # and these filesystem commands. A subagent whose frontmatter sets
  # acceptEdits gets the same, unseen by the session. Ask rules hold in every
  # mode and beat any allow rule, so these always ask. File tools are named
  # bare: a path rule on Write, NotebookEdit or MultiEdit never matches.
  fileTools = [ "Edit" "Write" "MultiEdit" "NotebookEdit" ];
  fileCommands = [ "mkdir" "touch" "rm" "rmdir" "mv" "cp" "sed" ];
  asked = fileTools ++ map (c: "Bash(${c} *)") fileCommands;
in
{
  inherit denied asked;

  # Claude Code: a managed-settings.d drop-in, so it merges with any other
  # admin policy instead of owning managed-settings.json. Managed values cannot
  # be overridden by user or project settings. Since v2.1.283 an interactive
  # session starts in auto mode, where a classifier approves actions instead of
  # the user; disabling it (and bypass) keeps every session asking. Nothing
  # disables acceptEdits; a managed defaultMode outranks one set in user or
  # local settings, and the ask rules take away what acceptEdits would skip.
  claude = {
    permissions = {
      defaultMode = "default";
      disableAutoMode = "disable";
      disableBypassPermissionsMode = "disable";
      deny = map (c: "Bash(${c} *)") denied;
      ask = asked;
    };
  };
}
