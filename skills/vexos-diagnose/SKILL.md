---
name: vexos-diagnose
description: >
  Diagnose why something failed on this VexOS (NixOS) machine: a crashed
  program (core dump), a systemd service that failed or will not start, or a
  failed `just rebuild` / nixos-rebuild. Use when a "Process crashed" or
  "diagnose with AI" prompt arrives, or when asked "why did X crash", "X keeps
  crashing", "service failed", "rebuild failed", "boot is slow", "something
  broke after an update". Triggers: segfault, SIGSEGV, SIGABRT, core dump,
  coredumpctl, failed unit, journalctl, evaluation error, build failure.
---

# Diagnosing a failure on VexOS

Work from evidence. The goal is an honest account of what happened, not a
plausible-sounding story.

**Diagnosis is read-only.** Do not fix, tidy, restart or reconfigure anything
while diagnosing. When you have found a fix, describe it. If the user wants it,
switch to the `vexos` skill, which covers how changes are made on this system
(edit Nix, dry-build, the user rebuilds).

If the prompt names a facts file (`/tmp/vexos-ai-diagnose.*`), read it first.
It holds what was collected when the problem was reported.

## Establish the facts

```bash
cat /etc/nixos/vexos-variant              # role and GPU
systemctl --failed; systemctl --user --failed
journalctl -b -p err --no-pager | tail -100
nixos-rebuild list-generations | tail -5  # what changed, and when
```

- **Service:** `systemctl status <unit>` and
  `journalctl -u <unit> -b --no-pager | tail -200`. Read the unit with
  `systemctl cat <unit>`; it lives in the Nix store, so the Nix module that
  generated it is where a fix would go.
- **Crash:** `coredumpctl info <pid>`. Note the command line as well as the
  backtrace; it usually shows what the program was working on.
  `coredumpctl list` shows whether this is a one-off or a pattern.
- **Rebuild failure:** read the error from the bottom up. NixOS evaluation
  errors name the option and file. Re-run the dry-build to reproduce:
  `nixos-rebuild dry-build --impure --flake "path:/etc/nixos#$(cat /etc/nixos/vexos-variant)" --show-trace`.

## Rule out the boring causes first

- Memory: `free -h`, and `journalctl -k -b | grep -i -E 'oom|killed process'`.
  A process killed by the OOM killer is not a bug in that process.
- Disk: `df -h / /nix /boot`. A full `/boot` breaks rebuilds.
- Network: failed fetches during a rebuild are often only that.

## Correlate against the timeline

On NixOS the most useful question is **what changed**. Compare the failure's
timestamp against:

- **Generations.** A failure that starts right after a rebuild points at that
  rebuild. `nix store diff-closures /nix/var/nix/profiles/system-{N-1,N}-link`
  lists the package versions that changed between two generations.
- **The journal** around that moment, for warnings from neighbouring units.
- **File mtimes**, for crashes: a file whose mtime matches the crash second is
  a strong hint at the trigger.

## Read the whole core, not just frame 0

Other thread stacks show what was in flight (loaders, IPC, GPU queues). Note
third-party code in the address space, such as plugins or out-of-tree drivers,
without blaming it unless evidence implicates it.

NixOS has no public debuginfod by default, so most frames will not symbolize.
When they do not, say so. **Never invent function names.** An unsymbolized
stack still shows which library each frame is in and which thread crashed.

A core is a copy of the process's memory and can hold passwords and tokens. If
you extract one, write it to a fresh `mktemp` path and delete it when done:

```bash
core=$(mktemp -t crash-XXXXXX.core); trap 'rm -f "$core"' EXIT
coredumpctl dump <pid> --output="$core"
gdb -q <executable> "$core" -batch -ex 'thread apply all bt'
```

## Report

1. What failed, and what it was doing at the time.
2. The most likely cause, separating clearly what the evidence **proves** from
   what you are **inferring**.
3. Whether data was lost, and where it can be recovered from.
4. Whether it will recur, and what would fix it. When the fix is "go back",
   say so: `just rollback`, or the previous generation in the boot menu.

If the cause is genuinely ambiguous, say so rather than guessing.

## Offer to silence crash notifications for that program

After explaining a crash, offer (never do it unprompted) to stop crash
notifications for **that one program**, and say how to undo it:

```bash
vexos-ai crash-mute '<program>'       # silence it
vexos-ai crash-mute '<program>' off   # let it notify again
vexos-ai crash-mute                   # list muted programs
```

Pass the executable's basename (from `coredumpctl info`), quoted. A program run
by an interpreter is keyed as the interpreter: muting `python3` silences every
Python program. Say so if that applies. A mute is not a fix; do not offer one in
place of a fix that is within reach.
