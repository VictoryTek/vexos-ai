# vexos-ai — launcher, picker, Claude accounts/usage, theme sync and
# "diagnose with AI" helpers for the VexOS AI assistant. Installed by
# nix/module.nix (programs.vexos-ai.enable); see bin/vexos-ai.sh for the commands.
#
# claude-code and opencode themselves are NOT runtime inputs: the module
# installs them system-wide so they can also be run directly, and vexos-ai
# finds them on PATH. nixos-rebuild, systemctl, journalctl and coredumpctl
# come from the running system for the same reason.
{ lib, writeShellApplication, makeDesktopItem, symlinkJoin
, coreutils, findutils, gnugrep, jq, curl, zenity, libnotify, glib, xdg-terminal-exec
, util-linux }:

let
  # The script with a given set of runtime inputs. writeShellApplication puts
  # them *in front of* PATH, so the contract check (checks.<system>.contract)
  # builds it without curl, zenity, libnotify and xdg-terminal-exec: the test
  # stubs for those must win. Same text, same shellcheck, same `set -euo`.
  mkScript = runtimeInputs: writeShellApplication {
    name = "vexos-ai";
    inherit runtimeInputs;
    text = builtins.readFile ../bin/vexos-ai.sh;
  };

  script = mkScript [
    coreutils findutils gnugrep jq curl zenity libnotify glib xdg-terminal-exec util-linux
  ];

  desktopItem = makeDesktopItem {
    name = "vexos-ai";
    desktopName = "VexOS Assistant";
    comment = "Ask an AI assistant to change or troubleshoot this system";
    exec = "vexos-ai";
    icon = "utilities-terminal";
    categories = [ "System" "Utility" ];
    keywords = [ "AI" "Claude" "OpenCode" "assistant" "help" "diagnose" ];
    actions = {
      panel = { name = "Accounts & usage"; exec = "vexos-ai panel"; };
      diagnose = { name = "Diagnose a problem"; exec = "vexos-ai diagnose"; };
      pick = { name = "Change AI tool"; exec = "vexos-ai pick"; };
    };
  };
in
symlinkJoin {
  name = "vexos-ai";
  paths = [ script desktopItem ];
  passthru = { inherit mkScript; };
  meta = {
    description = "VexOS AI assistant launcher and helpers";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "vexos-ai";
  };
}
