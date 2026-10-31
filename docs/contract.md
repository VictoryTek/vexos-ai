# vexos-ai: the machine contract

For callers that are not a person at a prompt: VexPortal (through the
`just ai-*` recipes in vexos-nix), the GNOME panel, tests. This is what they may
rely on. It holds for the bash CLI (`bin/vexos-ai.sh`) and for the Rust CLI that
replaces it (Phase 2): `tests/contract/*.bats` runs unchanged against both.

## Rules

- **Argv in, no prompts.** A command that has the argv it needs never prompts
  and never opens a dialog, even when a terminal and a display are both present
  (VexPortal's VTE has both).
- **Asking is a fallback**, and only for missing argv. It needs a terminal or a
  display. `--non-interactive` (or `VEXOS_AI_NONINTERACTIVE=1`) forbids it:
  missing input then exits 4. Only `pick` (with no agent) can ask.
- **Empty trailing arguments are missing arguments.** `just` passes `""` for a
  recipe parameter that was left unset, so `vexos-ai pick ""` and
  `crash-mute "" ""` mean `pick` and `crash-mute` (list).
- **Interactive by nature**, so they need a terminal: launching the assistant
  (bare `vexos-ai`, `--prompt`, `prompt`, `diagnose`, `crash`). Without a
  terminal they open one (`xdg-terminal-exec`) when there is a display, and
  exit 4 under `--non-interactive` or with no display.
- **Global flags** `--non-interactive` and `--json` may come before or after the
  command. After `prompt` or `--prompt`, everything is the prompt's text.
- **Output.** A human gets one line on stdout. Errors are one line on stderr,
  prefixed `vexos-ai:`, and leave stdout empty. With `--json`, a mutating command
  prints the new `status` document (and nothing else) on stdout, so a caller can
  refresh in one call. `status` and `crash-mute` (no name) with `--json` print
  the document too.

## Exit codes

| Code | Meaning | Examples |
|---|---|---|
| 0 | ok | |
| 1 | runtime failure | corrupt `agent` file |
| 2 | usage error: bad argv or value | unknown agent, unknown command, a removed command (`account …`, `panel`, `usage`) |
| 3 | not found | `claude` not installed |
| 4 | needs interaction, and none is allowed | `pick` with no agent and `--non-interactive`, or with no terminal or display |
| 5 | busy | reserved: nothing returns it now. It was `account remove` or `rename` while a session used the account |

## Commands

```
vexos-ai                                  open the assistant (in /etc/nixos)
vexos-ai --prompt "<text>"                … with a task
vexos-ai prompt <text…>                   same, words joined with spaces
vexos-ai pick [claude]                    set the agent; no argument → ask
vexos-ai status [--json] [--refresh]      state; --refresh is accepted and ignored
vexos-ai crash-mute                       list
vexos-ai crash-mute <prog> [on|off]       default on
vexos-ai crash-mute --off <prog>          same as '<prog> off'
vexos-ai crash-capture [on|off]           no argument → print "on" or "off"
vexos-ai crash <pid> [comm exe sig]       diagnose a core dump
vexos-ai diagnose [unit|rebuild]
vexos-ai theme
internal: crash-watch, theme-watch, welcome
```

Notes:

- **Removed:** `vexos-ai usage [account]` and the internal `usage-check` (and its
  `vexos-ai-usage` user timer) are gone. They are unknown commands now: exit 2,
  usage on stderr. See "Credentials and the network".
- **Removed:** the Claude accounts feature. `vexos-ai account` with any
  subcommand or none (`list`, `add`, `use`, `remove`, `rename`, `mode`) and the
  internal `panel` (the zenity accounts window, with its "Toggle autoswitch"
  button and the "Accounts" desktop action) exit 2 with one line on stderr,
  `vexos-ai: '<command>' was removed: …`, and nothing on stdout, also under
  `--json`. They never ask, open a terminal or touch a file. Claude always runs
  with its default sign-in (`~/.claude`; `CLAUDE_CONFIG_DIR` is never set). Sign
  in or switch accounts inside Claude Code itself (`/login`).
- `crash-capture off` creates a flag file and stops the watcher; `on` removes it
  and starts the watcher. If `systemctl --user` cannot reach a session, the
  setting is still saved and a warning goes to stderr (exit 0).
- `pick` accepts exactly the values VexPortal's catalog offers.

## `status --json`, schema 1

```json
{
  "schema": 1,
  "version": "1.0.0",
  "installed": true,
  "needsReboot": false,
  "agent": "claude",
  "workdir": "/etc/nixos",
  "accounts": [],
  "activeAccount": "main",
  "switching": { "mode": "manual", "threshold": 95 },
  "crash": { "capture": true, "muted": ["firefox"] }
}
```

- `schema` is an integer. It changes only on a breaking change. Additive keys do
  not bump it, so **consumers must ignore unknown keys**.
- `agent` is `null` until one is chosen (or if the file holds something else).
- The account keys are kept with **fixed values** so that schema 1 consumers
  keep parsing: `accounts` is always `[]`, `activeAccount` is always `"main"`,
  and `switching` is always `{ "mode": "manual", "threshold": 95 }`, whatever
  files older versions left. Do not rely on them: they may be dropped in a
  later schema.
- **Lost:** the per-account fields went with the list. A consumer that read
  `signedIn`, `plan`, `inUse` or `usage` from `accounts[]` finds no entry, so
  it gets nothing (`null` in jq). Nothing reports whether Claude is signed in
  or on which plan; Claude Code's own `/status` does.
- `needsReboot`: `sw/bin/vexos-ai` is in `/run/current-system` but not in
  `/run/booted-system` (the user services start at login).
- `installed` is always true when the CLI runs.
- `status` reads files only. It never goes to the network. `--refresh` is
  accepted for callers that still pass it and does nothing.

## Files

Paths and formats are unchanged; new state is additive. The released VexPortal
reads the files below directly and keeps working.

| Path | Format |
|---|---|
| `$XDG_CONFIG_HOME/vexos/ai/agent` | `claude`. A leftover `opencode` (OpenCode was removed) launches Claude and shows as `null` in `status --json` |
| `…/crash-ignore/<prog>` | empty file per muted program |
| `…/crash-capture-off` | flag file. The crash-watch unit has `ConditionPathExists=!%h/.config/vexos/ai/crash-capture-off` (systemd cannot see an `XDG_CONFIG_HOME` override) |

Versions with the accounts feature also wrote `…/claude-account` (the pointer
to the account for new sessions), `…/mode`, `…/threshold` and one Claude config
home per extra account, `$XDG_DATA_HOME/vexos/ai/claude-accounts/<label>/`.
**Nothing reads, writes, moves or deletes them now.** They stay on disk as
harmless leftovers, sign-in included; a user who wants an old account's
sign-in gone deletes its directory by hand. A caller that reads the pointer
directly must stop trusting it: the launcher no longer follows it.

Versions before the usage probe was removed also wrote
`$XDG_CACHE_HOME/vexos/ai/usage-<label>.json` and
`$XDG_STATE_HOME/vexos/ai/{probe-<label>.json,probe-<label>.lock,warned-<label>}`.
**Nothing writes or reads them now.** An old `usage-<label>.json` is frozen at
its last reading and no longer updates, so a caller that reads it directly (the
released VexPortal did) must stop trusting it. They are harmless leftovers.

## Credentials and the network

- **No network.** The CLI makes no network request. Nothing in it calls
  `api.anthropic.com` or any other endpoint, and `curl` is not one of its
  runtime inputs. The usage probe (a timer that read the login token and called
  Anthropic's undocumented OAuth usage endpoint) is removed. A caller that wants
  usage asks the agent itself: the assistant panel shows the 5-hour line that
  Claude Code's own `/usage` reports.
- **No credentials.** Nothing in this repo opens Claude's login file
  (`.credentials.json`): not the CLI, the module, the panel or the spike
  scripts. `credentials.bats` fails if any script or module names it. The
  CLI does not read, copy, move or delete credentials, and never runs a
  sign-in. A token is never printed, logged, sent anywhere or written to any
  file; a test plants one and greps every output and every file under `HOME`
  for it.

## What the safety policy covers, and what it does not

The agents are fenced by managed policy (`nix/policy.nix`, asserted by
`checks.policy`): applying a configuration is denied, and both agents ask before
they edit or run commands. That is a guardrail. What to know:

- The deny rules match **text**. `pkexec sh -c 'nixos-rebuild switch'` is not
  matched. The real gate is that applying needs root, and `sudo` or `pkexec`
  ask the user; the skills tell the agent to say what it is about to do first.
- Claude Code applies only the **highest** managed source that carries a policy
  key. If a user signs in with a Team or Enterprise organisation that pushes
  server-managed settings, VexOS's file policy is skipped, and nothing here can
  detect that. Check `/status` → "Setting sources" inside Claude Code.
- Every session **starts** asking: the policy sets Claude's `defaultMode` to
  `default`, which outranks user, project and local settings.
- Claude **always asks before writing a file** (`Edit`, `Write`, `MultiEdit`,
  `NotebookEdit`) **or running** `mkdir`, `touch`, `rm`, `rmdir`, `mv`, `cp` or
  `sed`, whatever the mode. These are managed `ask` rules, and an ask rule holds
  in every mode and outranks any allow rule. This closes two routes: switching
  to `acceptEdits` mid-session (Shift+Tab, or a client's `set_mode`), and a
  subagent whose definition (`.claude/agents/*.md`, project or user) sets
  `permissionMode: acceptEdits`, which used to edit unasked. What users notice:
  `acceptEdits` and "Yes, and don't ask again" no longer skip the prompt for
  those tools and commands. That includes read-only `sed -n`; other read-only
  commands such as `ls` and `cat` still run without asking.
- **OpenCode was removed.** Its one open route (a custom agent the user
  selects, which bypassed the managed denies) no longer exists here. Claude
  Code is the only assistant, and no OpenCode policy is installed.

## Testing it

`tests/contract/*.bats` is the executable form of this document: black-box, a
fake `HOME`, fake system profiles, and stub `claude`,
`curl`, `systemctl`, `zenity`, `xdg-terminal-exec`, `notify-send` and `gsettings`
first on `PATH`. The stubs log their calls, so "never opens a dialog" is an
assertion; the `curl` stub is a tripwire that fails and logs if anything calls
it. Terminal cases run under `script(1)` for a real pty. `credentials.bats`
also scans the source tree (`VEXOS_AI_SRC`, by default this checkout). It runs
as `checks.<system>.contract`; by hand:

```sh
nix develop -c env VEXOS_AI_BIN=<path to a vexos-ai built without zenity,
  libnotify, xdg-terminal-exec in its runtime inputs> bats tests/contract
```

By hand, `claude` must not be on `PATH`: the tests install its
stub only where they want an agent, and a test that expects "not installed"
would otherwise start the real one.

(`packages.<system>.default.passthru.mkScript [ … ]` builds that variant. The
packaged `vexos-ai` puts its own `zenity`, `libnotify` and `xdg-terminal-exec`
in front of `PATH`, which would hide the stubs.)
