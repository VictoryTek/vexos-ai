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
vexos-ai status [--json] [--refresh]      state; --refresh re-reads usage
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
vexos-ai usage [account]                  the usage cache record (see Files)
vexos-ai theme
internal: panel, crash-watch, usage-check, theme-watch, welcome
```

Notes:

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
      "usage": {
        "ok": true,
        "session": 12.0, "sessionResetsAt": "2026-10-07T18:36:41+00:00",
        "weekly": 40.0,  "weeklyResetsAt":  "2026-10-10T15:36:41+00:00",
        "scoped": [ { "label": "Opus Weekly", "percent": 30.0, "resetsAt": "2026-10-10T15:36:41+00:00" } ],
        "fetchedAt": "2026-10-07T15:36:41+00:00",
        "stale": false,
        "why": null
      }
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
  `usage` is `null` until the first reading.
- Percentages are 0-100 floats. A source that reports 0-1 fractions is converted.
  A window whose `…ResetsAt` has passed reads 0.
- `usage.ok` is false when there are no numbers at all; `session`, `weekly` and
  the reset times are then `null` and `why` says why. `ok: true` with
  `stale: true` means the numbers are the last good ones and `why` says why they
  could not be refreshed (rate limited, expired sign-in …).
- `signedIn` is false only when the account has no refresh token (or an expired
  access token and nothing to renew it). An expired *access* token is routine:
  `signedIn` stays true and `usage.why` is "start Claude Code to refresh it".
- `plan` comes from the account's credentials ("Max 5x", "Pro"), or is `null`.
- `inUse`: a running Claude Code process of this user has the account as its
  `CLAUDE_CONFIG_DIR` (none set means `main`).
- `needsReboot`: `sw/bin/vexos-ai` is in `/run/current-system` but not in
  `/run/booted-system` (the user services start at login).
- `installed` is always true when the CLI runs.
- Plain `status` reads files only. It never goes to the network. Only
  `--refresh` (and `usage`) re-read usage, and they respect backoff.

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
| `$XDG_CACHE_HOME/vexos/ai/usage-<label>.json` | `ok` (bool) `session` `weekly` (numbers) `sessionReset` `weeklyReset` `why` (strings, `why` is `""` when fine), plus `fetchedAt` `plan` `scoped` `stale` |
| `$XDG_STATE_HOME/vexos/ai/probe-<label>.json` | `{nextAt, lastAt, failures}`, epoch seconds. Never a token |
| `$XDG_STATE_HOME/vexos/ai/probe-<label>.lock` | `flock` file |

## Usage probing

The usage endpoint is undocumented and rate-limited, so:

- **Token handling.** The access token is read from `.credentials.json`, sent to
  curl on stdin for one `Authorization` header, and dropped. It is never in argv,
  logs, the cache, state, error text or `status`. A test greps every file the CLI
  writes and every output for it.
- **Single flight.** One probe per account at a time (`flock`), whichever of the
  timer, the CLI, VexPortal or the panel asks. A caller that does not force
  skips; `--refresh` and `usage` wait for the one in flight.
- **Intervals.** The active account every 10 minutes, every 3 when within 15
  points of the threshold, other accounts every 60. `--refresh` skips the
  interval but never a backoff, and never probes twice within 15 seconds.
- **Backoff.** On 429, `Retry-After` (seconds) is honoured. Other failures
  (including 401, 403, 5xx and an unknown payload) back off 1, 2, 4 … 60 minutes.
  A transport error retries in a minute and does not count as a failure.
- **Degrade.** The last good numbers are kept, with `stale: true` and a short
  reason. An unknown payload is "usage format changed", never a crash.
- Scoped (per-model) limits are read from `seven_day_<model>` keys in the
  payload, labelled "<Model> Weekly". The payload is undocumented, so this is
  best effort and `scoped` may be empty.

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
- OpenCode: an agent the user defines themselves gets the global rules first and
  its own after them, so it can allow what the built-in agents ask about.

## Testing it

`tests/contract/*.bats` is the executable form of this document: black-box, a
fake `HOME`, a fake `/proc` and system profiles, and stub `claude`, `opencode`,
`curl`, `systemctl`, `zenity`, `xdg-terminal-exec`, `notify-send` and `gsettings`
first on `PATH`. The stubs log their calls, so "never opens a dialog" is an
assertion. Terminal cases run under `script(1)` for a real pty. It runs as
`checks.<system>.contract`; by hand:

```sh
nix develop -c env VEXOS_AI_BIN=<path to a vexos-ai built without curl, zenity,
  libnotify, xdg-terminal-exec in its runtime inputs> bats tests/contract
```

(`packages.<system>.default.passthru.mkScript [ … ]` builds that variant. The
packaged `vexos-ai` puts its own `curl` in front of `PATH`, which would hide the
stub.)
