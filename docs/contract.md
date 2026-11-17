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
| 3 | not found | the chosen agent (`claude`, `codex` or `opencode`) is not installed |
| 4 | needs interaction, and none is allowed | `pick` with no agent and `--non-interactive`, or with no terminal or display |
| 5 | busy | reserved: nothing returns it now. It was `account remove` or `rename` while a session used the account |

## Commands

```
vexos-ai                                  open the assistant (in /etc/nixos)
vexos-ai --prompt "<text>"                … with a task
vexos-ai prompt <text…>                   same, words joined with spaces
vexos-ai pick [claude|codex|opencode]    set the agent; no argument → ask
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
- `agent` is `"claude"`, `"codex"` or `"opencode"`, and `null` until one is chosen (or if the file holds something else).
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
| `$XDG_CONFIG_HOME/vexos/ai/agent` | `claude`, `codex` or `opencode`, exactly (first line, no trimming or case folding). Anything else shows as `null` in `status --json` and the window falls back to Claude |
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

- **The CLI: no network.** The CLI makes no network request. Nothing in it calls
  `api.anthropic.com` or any other endpoint, and `curl` is not one of its
  runtime inputs. The old usage probe (a timer in the CLI that read the login
  token) stays removed.
- **The CLI: no credentials.** The CLI, the module, the assistant window, the
  Shell extension and the spike scripts never open Claude's login file
  (`.credentials.json`). `credentials.bats` fails if any of them names it. The
  CLI does not read, copy, move or delete credentials, and never runs a
  sign-in. A token is never printed, logged, sent anywhere or written to any
  file; a test plants one and greps every output and every file under `HOME`
  for it.
- **The one exception: the usage service** (below). Exactly one source file in
  the repo, `crates/vexos-ai-usage/src/credentials.rs`, opens the login file,
  and only after the user has switched plan limits on.

## Usage service

`vexos-ai-usage` (`crates/vexos-ai-usage`) gives the top-bar panel Claude's and
Codex's plan limits (Session, Weekly and any model-scoped windows). It is not part of
the CLI and has no argv contract: it is a D-Bus activated program that the
Shell extension asks by calling the `refresh` or `refresh-now` action of
`io.github.victorytek.VexosAssistant.Usage`. It exits after 30 s idle.

- **Off until the user says so.** The GSettings key `usage-enabled` (schema
  `org.gnome.shell.extensions.vexos-assistant`, default `false`) is read
  before anything else. A missing schema or key means off. Off reads no file and
  makes no request, and the record is cleared. The panel's "Show plan limits"
  button is the only thing that turns it on, and says what it reads.
- **What it reads and sends.** On, it reads the access token and plan from
  Claude Code's saved sign-in (under `CLAUDE_CONFIG_DIR`, default `~/.claude`)
  and the CLI's profile, and sends the token in the `Authorization` header of
  one `GET https://api.anthropic.com/api/oauth/usage` request, with the
  `anthropic-beta: oauth-2025-04-20` header. The host and path are constants.
  The request is https only, follows no redirect, times out after 10 s and reads
  at most 64 KiB. It never refreshes, rewrites or deletes the sign-in: only the
  Claude CLI renews the token, so a lapsed one is reported ("paused" or "sign-in
  expired"), not fixed.
- **Codex's limits, asked of Codex.** On, and only if a `codex` is on the
  service's PATH (the module puts the Codex it was given there), the service
  starts `codex app-server` (JSON lines on stdin and stdout), sends
  `initialize`, `account/read` and, when the account is a ChatGPT plan,
  `account/rateLimits/read`, and kills it. Codex uses its own login: this
  service reads no Codex file, holds no Codex token and copies no email, and
  keeps from the answer only the plan word and each window's `usedPercent`,
  `windowDurationMins` and `resetsAt`. The request ends after 15 s and reads at
  most 256 KiB. A window's length names it (300 minutes Session, 10080 Weekly,
  otherwise "N-hour" or "N-day"); a window Codex does not report is absent, not
  zero. With no `codex`, or with the switch off, there is no `codex` section.
  Not run live here: the request shapes come from Codex's published protocol
  source, so check them against the installed Codex.
- **What it writes.** `$XDG_STATE_HOME/vexos-ai/usage.json` (mode 0600,
  replaced atomically), schema 1: `updatedAt`, `fetchedAt`, `probedAt`,
  `nextAt`, `failures`, `enabled`, `status`, `plan`, `stale` and `windows`
  (`id`, `label`, `percent` 0 to 100, `resetsAt` in ms, 0 when unknown). Those
  fields are Claude's. When Codex is installed there is also a `codex` object
  with `status`, `plan`, `stale`, `fetchedAt`, `nextAt` and `windows`, in the same
  shapes. `status` is one of `ok`, `disabled`, `waiting-for-auth`, `paused`,
  `signin-expired`, `unavailable`, `rate-limited`, `unreachable`,
  `format-changed`, `no-plan` (Codex signed in with an API key, which has no plan
  limits). Consumers must ignore unknown keys; a new schema number is a
  breaking change. The file never holds a token, a header, an email or other text
  from the other side beyond a cleaned model label.
- **Pacing.** A request is never made twice within 15 s unless forced
  (`refresh-now`). After a 429 the service waits `Retry-After`, else 1, 2, 4 ...
  60 minutes; a refused sign-in waits 15 minutes; nothing answering retries after
  1 minute without counting as a failure. `refresh-now` skips that 15 s interval and the
  15-minute wait after a refused sign-in (so signing in again takes effect at
  once), but never any other backoff. On a failure the last numbers stay, marked `stale`; a window
  whose reset has passed reads 0%. Codex has its own pacing: the same 15 s
  interval, five minutes after it failed to answer, and a short pause after
  "waiting for a sign-in" that a forced refresh skips.
- **The token is a `Secret`.** Its `Debug` prints `[redacted]`; it has no
  `Display`, `Clone` or serialisation, and tests plant one and check that no
  record, file or print holds it.

This is Anthropic's undocumented endpoint, the one Claude Code's own `/usage`
uses, and Anthropic's terms restrict third-party products from offering rate
limits. It is therefore opt-in, isolated in one crate and one file, and may stop
working without notice (an unknown answer shows "format-changed", never a wrong
figure).

## What the safety policy covers, and what it does not

The agents are fenced by managed policy (`nix/policy.nix`, asserted by
`checks.policy`): applying a configuration is denied, and every agent asks before
it edits or runs commands. That is a guardrail. What to know:

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
- **Codex** reads `/etc/codex/requirements.toml`, an admin-enforced layer that
  user and project config cannot override: approvals are `untrusted` or
  `on-request` (never `never`; `on-request` is what codex-acp sends in its
  `read-only` mode), the reviewer is always `user` (never Codex's auto
  reviewer), the sandbox is `read-only` (a write needs an approval the user
  answers), and each applier, bare or behind `sudo` or `pkexec`, is a
  `forbidden` rule. The other codex-acp modes (`agent`, `agent-full-access`)
  ask for settings these requirements refuse. The window starts it in the ACP mode `read-only` and resets
  any other (`agent`, `agent-full-access`, `auto`, `full-access`).
- **OpenCode** reads `/etc/opencode/opencode.json`, written in the **2.x** format
  (1.x's `permission`/`bash`/`task` keys are not used; nixpkgs-unstable ships
  2.x, and the module refuses anything older). `permissions` makes edits and
  shell commands `ask` and denies the `subagent` action, globally and on the
  `build` agent the window pins (an agent's own rules are read after the global
  ones). `policies` are the hard guarantees: each applier, bare or behind `sudo`
  or `pkexec`, and starting any subagent are denied after every agent rule and
  saved approval. The asks are not hard: a user's own "allow always" settles one
  (OpenCode appends the system file's agent rules after the user's and the
  project's, so their allow rules for `build` do not outrank ours). The window resets any agent other than `build`. The format comes from
  OpenCode's published 2.x docs; no 2.x OpenCode was run for it.
- **Not run live yet:** the Codex and OpenCode policies are asserted as rendered
  files and the window's mode pinning is tested against a fake agent, but no real
  codex-acp 2.x or OpenCode 2.x session was run for this. Run `spike/acp` against
  each on a NixOS host and check that every edit and command asks, before relying
  on it.

## Testing it

`tests/contract/*.bats` is the executable form of this document: black-box, a
fake `HOME`, fake system profiles, and stub `claude`, `codex`, `opencode`,
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

By hand, none of `claude`, `codex` or `opencode` may be on `PATH`: the tests install
their stubs only where they want an agent, and a test that expects "not installed"
would otherwise start the real one.

(`packages.<system>.default.passthru.mkScript [ … ]` builds that variant. The
packaged `vexos-ai` puts its own `zenity`, `libnotify` and `xdg-terminal-exec`
in front of `PATH`, which would hide the stubs.)
