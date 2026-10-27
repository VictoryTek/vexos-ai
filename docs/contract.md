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
  missing input then exits 4. Only `pick` (with no agent) and `account add` (with
  no label) can ask.
- **Empty trailing arguments are missing arguments.** `just` passes `""` for a
  recipe parameter that was left unset, so `vexos-ai pick ""`,
  `crash-mute "" ""` and `account mode auto ""` mean `pick`, `crash-mute`
  (list) and `account mode auto`.
- **Interactive by nature**, so they need a terminal: `account add` (browser
  sign-in through `claude auth login`) and launching the assistant (bare
  `vexos-ai`, `--prompt`, `prompt`, `diagnose`, `crash`). Without a terminal they
  open one (`xdg-terminal-exec`) when there is a display, and exit 4 under
  `--non-interactive` or with no display.
- **Global flags** `--non-interactive` and `--json` may come before or after the
  command. After `prompt` or `--prompt`, everything is the prompt's text.
- **Output.** A human gets one line on stdout. Errors are one line on stderr,
  prefixed `vexos-ai:`, and leave stdout empty. With `--json`, a mutating command
  prints the new `status` document (and nothing else) on stdout, so a caller can
  refresh in one call. `account add --json` sends the sign-in chatter to stderr.
  `status`, `account list` and `crash-mute` (no name) with `--json` print the
  document too.

## Exit codes

| Code | Meaning | Examples |
|---|---|---|
| 0 | ok | |
| 1 | runtime failure | sign-in did not finish; corrupt `agent` file |
| 2 | usage error: bad argv or value | unknown agent, bad label, threshold 0 or 101, taken name, unknown command, `remove main` |
| 3 | not found | no such account; `claude` or `opencode` not installed |
| 4 | needs interaction, and none is allowed | `pick` with no agent and `--non-interactive`, or with no terminal or display; `account add` off a terminal under `--non-interactive` |
| 5 | busy | `account remove` or `rename` while a running Claude Code session uses the account |

## Commands

```
vexos-ai                                  open the assistant (in /etc/nixos)
vexos-ai --prompt "<text>"                … with a task
vexos-ai prompt <text…>                   same, words joined with spaces
vexos-ai pick [claude|opencode]           set the agent; no argument → ask
vexos-ai status [--json] [--refresh]      state; --refresh is accepted and ignored
vexos-ai account list
vexos-ai account add <label>              interactive sign-in (terminal)
vexos-ai account use <label|next>         main, a label, or the next account
vexos-ai account remove <label>           not main; exit 5 while in use
vexos-ai account rename <label> <new>     not main; exit 5 while in use
vexos-ai account mode [manual|auto] [pct] pct 1-100; no argument → report
vexos-ai crash-mute                       list
vexos-ai crash-mute <prog> [on|off]       default on
vexos-ai crash-mute --off <prog>          same as '<prog> off'
vexos-ai crash-capture [on|off]           no argument → print "on" or "off"
vexos-ai crash <pid> [comm exe sig]       diagnose a core dump
vexos-ai diagnose [unit|rebuild]
vexos-ai theme
internal: panel, crash-watch, theme-watch, welcome
```

Notes:

- **Removed:** `vexos-ai usage [account]` and the internal `usage-check` (and its
  `vexos-ai-usage` user timer) are gone. They are unknown commands now: exit 2,
  usage on stderr. See "Credentials and the network".
- `account use` with no target means `next`. `next` cycles `main`, then the
  labels in name order, then `main`.
- Labels are 1-32 of `A-Za-z0-9_-`. `main` and `next` are reserved. `account use`
  and the others take only a label, never a path.
- `account mode` validates both values before it writes either.
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
  "accounts": [
    {
      "label": "main",
      "active": true,
      "signedIn": true,
      "plan": "Max 5x",
      "inUse": false,
      "usage": null
    }
  ],
  "activeAccount": "main",
  "switching": { "mode": "manual", "threshold": 95 },
  "crash": { "capture": true, "muted": ["firefox"] }
}
```

- `schema` is an integer. It changes only on a breaking change. Additive keys do
  not bump it, so **consumers must ignore unknown keys**.
- `agent` is `null` until one is chosen (or if the file holds something else).
- `usage` is **always `null`**. Earlier versions filled it from a usage probe,
  which is removed (see "Credentials and the network"). The key stays so that
  schema 1 consumers, which already handle `null`, keep parsing. Do not rely on
  it: it may be dropped in a later schema.
- `signedIn` is false only when the account has no refresh token (or an expired
  access token and nothing to renew it). An expired *access* token is routine:
  `signedIn` stays true.
- `plan` comes from the account's credentials ("Max 5x", "Pro"), or is `null`.
- `inUse`: a running Claude Code process of this user has the account as its
  `CLAUDE_CONFIG_DIR` (none set means `main`).
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
| `$XDG_CONFIG_HOME/vexos/ai/agent` | `claude` or `opencode` |
| `…/claude-account`, `…/mode`, `…/threshold` | first line |
| `…/crash-ignore/<prog>` | empty file per muted program |
| `…/crash-capture-off` | flag file. The crash-watch unit has `ConditionPathExists=!%h/.config/vexos/ai/crash-capture-off` (systemd cannot see an `XDG_CONFIG_HOME` override) |
| `$XDG_DATA_HOME/vexos/ai/claude-accounts/<label>/` | Claude config home |

Versions before the usage probe was removed also wrote
`$XDG_CACHE_HOME/vexos/ai/usage-<label>.json` and
`$XDG_STATE_HOME/vexos/ai/{probe-<label>.json,probe-<label>.lock,warned-<label>}`.
**Nothing writes or reads them now.** An old `usage-<label>.json` is frozen at
its last reading and no longer updates, so a caller that reads it directly (the
released VexPortal did) must stop trusting it. `account remove` deletes the
ones of the account it removes; any others are harmless leftovers.

## Credentials and the network

- **No network.** The CLI makes no network request. Nothing in it calls
  `api.anthropic.com` or any other endpoint, and `curl` is not one of its
  runtime inputs. The usage probe (a timer that read the login token and called
  Anthropic's undocumented OAuth usage endpoint) is removed. A caller that wants
  usage asks the agent itself: the assistant panel shows the 5-hour line that
  Claude Code's own `/usage` reports.
- **What it still reads.** `status` reads each account's `.credentials.json` for
  `signedIn` and `plan` only: whether a refresh token exists, whether the access
  token has not expired, and the subscription and rate-limit tier labels. A
  token is never printed, logged, sent anywhere or written to any file; a test
  greps every output and every file under `HOME` for it.
- **What it never does.** It does not copy credentials. `account add` has Claude
  sign in to the new account's own config directory (`claude auth login`),
  `account rename` moves that directory and `account remove` deletes it.
- **Switching mode and threshold.** `account mode` still stores `mode` and
  `threshold` (and `status` still reports them as `switching`), but they have no
  effect: they only ever steered the removed usage warning and auto-switch.

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
  `default` and OpenCode's `default_agent` to `build`, which outrank user,
  project and local settings.
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
- One route is **not closed**, and needs a deliberate user action. Managed
  config has no lock for it. **OpenCode custom agents**: an agent defined with
  its own `allow` rules (in `.opencode/agents/`, a project `opencode.json` or
  `~/.config/opencode/`) gets the global rules first and its own after them.
  Once the user selects it (Tab in the TUI, `--agent`, or a client's mode
  choice), it bypasses the managed denies too: `nixos-rebuild switch` runs
  unasked. The assistant core (`crates/vexos-ai-core`) keeps OpenCode on
  `build` and offers no other agent, so the route doesn't apply there. The
  OpenCode terminal agent is unchanged.

## Testing it

`tests/contract/*.bats` is the executable form of this document: black-box, a
fake `HOME`, a fake `/proc` and system profiles, and stub `claude`, `opencode`,
`curl`, `systemctl`, `zenity`, `xdg-terminal-exec`, `notify-send` and `gsettings`
first on `PATH`. The stubs log their calls, so "never opens a dialog" is an
assertion; the `curl` stub is a tripwire that fails and logs if anything calls
it. Terminal cases run under `script(1)` for a real pty. It runs as
`checks.<system>.contract`; by hand:

```sh
nix develop -c env VEXOS_AI_BIN=<path to a vexos-ai built without zenity,
  libnotify, xdg-terminal-exec in its runtime inputs> bats tests/contract
```

(`packages.<system>.default.passthru.mkScript [ … ]` builds that variant. The
packaged `vexos-ai` puts its own `zenity`, `libnotify` and `xdg-terminal-exec`
in front of `PATH`, which would hide the stubs.)
