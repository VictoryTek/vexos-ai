# The assistant's safety policy, as the managed agent configs (Claude Code,
# Codex, OpenCode) that nix/module.nix installs. Kept apart from the module so `checks.policy` can
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

  # OpenCode 2.x (the format nixpkgs-unstable ships; 1.x used `permission`,
  # `bash` and `task` and is not supported). `permissions` is an ordered list
  # where the last matching rule wins: edits and shell commands ask, and an agent
  # may not start a subagent, because a subagent runs under rules of its own that
  # this file cannot reach. `policies` are hard denies that apply after every
  # agent rule and saved approval, and that no user or project config can lift.
  opencodePermissions = [
    { action = "edit"; resource = "*"; effect = "ask"; }
    { action = "shell"; resource = "*"; effect = "ask"; }
    { action = "subagent"; resource = "*"; effect = "deny"; }
  ];
  opencodePolicies = map
    (c: { action = "tool.use"; resource = "shell:${c} *"; effect = "deny"; })
    denied
    ++ [ { action = "tool.use"; resource = "subagent:*"; effect = "deny"; } ];

  # Codex: one forbidden prefix rule per applier and prefix. A command is its
  # tokens, so "sudo" and "pkexec" are tokens of their own. Codex also matches
  # these inside a shell wrapper it can parse.
  codexRule = cmd: {
    pattern = map (token: { inherit token; }) (lib.splitString " " cmd);
    decision = "forbidden";
    justification = "VexOS agents never apply a configuration: hand `${cmd}` to the user, who runs it (`just rebuild`), and use `nixos-rebuild dry-build` to check.";
  };
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

  # Codex: /etc/codex/requirements.toml, the admin-enforced layer that user and
  # project config cannot override. Approvals always go to the user, never to
  # Codex's auto reviewer, and never "never": `untrusted` (the default for a
  # plain `codex`: only known read-only commands run unasked) and `on-request`
  # (what codex-acp sends every turn in its `read-only` mode, the one mode the
  # core allows). The sandbox is read-only, so a write needs an approval the user
  # answers, and the appliers are forbidden outright. Rendered to TOML by the
  # module and the check.
  codex = {
    allowed_approval_policies = [ "untrusted" "on-request" ];
    allowed_approvals_reviewers = [ "user" ];
    allowed_sandbox_modes = [ "read-only" ];
    rules.prefix_rules = map codexRule denied;
  };

  # OpenCode: /etc/opencode/opencode.json, the system config, which OpenCode
  # ranks above global and project config. The window starts it in the `build`
  # agent (the core pins that), so the same asks are set on that agent too: an
  # agent's own rules are read after the global ones, and a user's or project's
  # `agents.build.permissions` would otherwise come last. Only `policies` are
  # hard guarantees (the appliers, and starting a subagent); an ask can still be
  # settled by the user's own "allow always". Written for 2.x from its published
  # docs and not run live here: see docs/contract.md.
  opencode = {
    permissions = opencodePermissions;
    agents.build.permissions = opencodePermissions;
    policies = opencodePolicies;
  };
}
