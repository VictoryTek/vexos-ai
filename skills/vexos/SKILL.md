---
name: vexos
description: >
  REQUIRED for any request to change, install, enable, disable, configure or
  explain something about this VexOS (NixOS) machine: packages, services,
  features (gaming, development, virtualization, VPN, Sunshine), hostname,
  bootloader, desktop environment, kernel, drivers, users, network, storage.
  Triggers: "install", "enable", "turn on/off", "add a program", "change my
  system", "why is X set", nixos, /etc/nixos, just recipes, rebuild. For "why
  did X crash / fail / not start" use the vexos-diagnose skill instead.
---

# VexOS

VexOS is a NixOS configuration. The whole operating system is declared in Nix
and built from it, so **the system is changed by editing Nix files, never by
editing the running system**. A change takes effect only when the user runs
`just rebuild`. That step belongs to the user, never to you.

## Ground rules

1. **You never apply changes.** Do not run `nixos-rebuild switch`,
   `nixos-rebuild boot`, `just rebuild`, `just switch`, `just update` or
   `nix flake check`. These are blocked for you and must stay that way. Edit,
   validate, then hand over.
2. **Edit only the host side-files in `/etc/nixos`** (listed below). The rest
   of the configuration is upstream VexOS, pulled from GitHub on every build:
   read it freely, never try to change it. `/nix/store` is read-only.
3. **Never touch** `hardware-configuration.nix`, `vexos-variant`,
   `user-override.nix`, `stateless-user-override.nix`, `host.nix`, anything
   under `/etc/nixos/secrets/`, or any `system.stateVersion` value.
4. **Dotfiles are usually owned by Home Manager.** A file in `~/.config` that is
   a symlink into `/nix/store` is rewritten on every rebuild; editing it is
   lost. Say so and make the change in Nix instead.
5. **Privilege.** Your shell has no terminal for a password prompt, so `sudo`
   fails. To write a root-owned file use `pkexec`, which shows the user a
   graphical password dialog, e.g.
   `printf '%s' "$content" | pkexec tee /etc/nixos/local.nix >/dev/null`.
   Each `pkexec` is a request for the user's approval: batch your edits, and
   say what you are about to write before you ask.

## Know the machine first

```bash
cat /etc/nixos/vexos-variant          # e.g. vexos-desktop-amd  (role-gpu)
just feature list                     # optional features and their state
just --list                           # every recipe the user can run
ls /etc/nixos                         # which side-files exist
nixos-rebuild list-generations | tail -5
```

Roles: `desktop`, `stateless`, `server`, `headless-server`, `htpc`, `vanilla`.
GPU variants: `amd`, `nvidia`, `nvidia-legacy580`, `intel`, `vm`.

To read the upstream source this machine is built from:

```bash
nix flake metadata /etc/nixos --json \
  | jq -r '.locks.nodes."vexos-nix".locked | "\(.owner)/\(.repo) @ \(.rev)"'
nix flake archive /etc/nixos --json | jq -r '.inputs."vexos-nix".path'   # local store copy
```

Modules live under `modules/`, the role files are `configuration-<role>.nix`,
custom options are named `vexos.*`. Search the store copy with `grep -rn` to
find what an option does before suggesting it.

## Where a change goes (in this order)

1. **A `just` recipe, if one exists.** It knows the right file and format.
   Run it only if it merely edits files. If it also rebuilds (most do not; read
   the recipe with `just --show <name>` first), tell the user to run it instead.
   Examples: `just feature enable gaming`, `just set-hostname <name>`,
   `just service enable <service>` (server roles), `just bootloader switch`.
2. **The side-file that owns the setting:**

   | File | Holds |
   |---|---|
   | `features.nix` | `vexos.features.<name>.enable` toggles: gaming, development, print3d, virtualization, sunshine, vpn, ai; `vexos.desktop.environment` |
   | `server-services.nix` | `vexos.server.<service>.enable` (server roles) |
   | `hostname.nix` | `networking.hostName` |
   | `bootloader.nix` | bootloader choice |
   | `hardware-local.nix` | ASUS / OpenRGB hardware options |
   | `local.nix` | **everything else**: extra packages, udev rules, services, tweaks |

3. **`/etc/nixos/local.nix`** for anything that is not a feature or recipe. It is
   an ordinary NixOS module owned by the user and never overwritten:

   ```nix
   { config, pkgs, lib, ... }:
   {
     environment.systemPackages = with pkgs; [ htop ];
   }
   ```

   Before using it, confirm this machine loads it:
   `grep -q '\./local\.nix' /etc/nixos/flake.nix`. If that fails, the wrapper
   predates local.nix: tell the user that `vexos-update` (or the Up app) adds it
   automatically on the next update, and stop.

   `pkgs.unstable.<name>` is available for packages newer than the stable
   channel. Find packages with `nix search nixpkgs <term>` and options with
   `man configuration.nix` or the mcp-nixos tool when it is configured.

Keep changes minimal and commented. Prefer a VexOS option (`vexos.*`) over
setting the underlying NixOS option directly, because the VexOS one is what the
rest of the config expects.

## Validate every change

```bash
variant=$(cat /etc/nixos/vexos-variant)
nixos-rebuild dry-build --impure --flake "path:/etc/nixos#$variant"
```

This evaluates the whole system and changes nothing. If it fails with a
permission error, the host has root-only files in `/etc/nixos`. Run the same
command through `pkexec` instead. If it fails on your change, fix it. If it
fails for a reason unrelated to your change, report that and stop.

## Hand over

Finish every change with:

1. What you changed, file by file, and why (`diff` old against new if useful).
2. The dry-build result.
3. The exact command for the user: **`just rebuild`**.
4. How to undo: revert the file, or after rebuilding, `just rollback` (or
   choose the previous generation in the boot menu).

Never say a change is applied, active or done before the user has rebuilt.
