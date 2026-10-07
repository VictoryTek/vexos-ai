# vexos-ai: refined, robust, GNOME-native — spec

Status: **decided 2026-10-06** (A: VexPortal deep link · B: Rust, Phase 1 in bash ·
C: both agents ask · D: top-bar extension last). Phase 0 done; Phase 1 next.

Researched 2026-10-06 against:

- this repo at `ea7af3b`
- VexPortal `f524cd3` (`src/system/ai.rs`, `src/ui/pages/ai.rs`,
  `ai_assistant_page_spec.md`, `catalog.toml`)
- vexos-nix `040e63e` (`modules/ai.nix`, `gnome*.nix`, `justfile`,
  `template/etc-nixos-flake.nix`)
- Omarchy `omacom/omarchy@81145eb` (2026-10-05) and omarchy.org/manual/ai
- Claude Code docs (managed settings, permission modes), OpenCode docs (skills,
  themes, permissions, managed config), GNOME developer docs (search providers),
  gjs.guide (GNOME 50 port notes), gtk4-rs book (D-Bus activation)

Pinned platform (vexos-nix `nixos-26.05` @ `b253099`, the same rev as this repo's
lock): **GNOME Shell 50.5**, GTK 4.22.4, libadwaita 1.9.3, rustc 1.95, bats 1.12.
Agents come from unstable: claude-code **2.1.287**, opencode **1.18.34**.

---

## 0. Findings that change the plan

Four of these are safety issues. The safety rule is fixed, so they come before
any feature work (Phase 0).

1. **Claude Code no longer starts in an asking mode.** From v2.1.283, *auto mode*
   is the built-in starting permission mode for interactive terminal sessions. In
   auto mode a classifier approves actions instead of the user. vexos-nix ships
   2.1.287, so every VexOS Claude session already starts in auto. Deny rules still
   apply, but "the agent asks first" is no longer true. Fix: add
   `permissions.disableAutoMode: "disable"` and
   `permissions.disableBypassPermissionsMode: "disable"` to the managed drop-in.
   Both are lock keys, so they also survive a merge with other managed sources.
2. **OpenCode never asked.** Its defaults are permissive: most permissions are
   `"allow"`, and only `external_directory` and `doom_loop` ask. Our managed
   `/etc/opencode/opencode.json` added denies and nothing else. Tested against
   opencode 1.18.34 (`opencode debug agent <name>`): the rules run in this order,
   and the last match wins:
   1. the built-in agent rules
   2. the global `permission`
   3. each agent's own `agent.<name>.permission`

   So a user's `agent.build.permission.bash."*" = "allow"` beat **both** the ask
   and the deny list. A global `edit = "ask"` also turns the plan agent's edit
   deny into ask. Fix (done in Phase 0, `nix/policy.nix`): `edit = "ask"` and
   `bash = {"*": "ask", …denies}`, set globally **and** under `build`,
   `general`, `plan` and `explore`. Result for every built-in agent, with or
   without a hostile user config: appliers are denied, everything else asks.
   The user's own explicit allows for other commands still apply. Residual risk:
   an agent the user defines themselves gets the global rules first and its own
   rules after them.
3. **The Claude file policy can be skipped without warning.** Claude Code applies
   only the *highest* managed source that carries a policy key (`first-wins`).
   Remote, server-managed settings rank above `/etc/claude-code`. If a user signs
   in with a Team or Enterprise org that has server-managed settings, our deny
   drop-in is silently ignored. A lower source cannot opt into `merge`, and no
   supported CLI call reports which source won (only the interactive `/status`
   does). So we cannot fix or reliably detect it from here. Instead,
   `docs/contract.md` and the README document it as residual risk, and the
   welcome text tells work-org users to check `/status` → "Setting sources". The
   real gate stays root: applying needs `sudo` or `pkexec`, and the user answers
   both.
4. **The deny list matches text, not intent.** For example,
   `pkexec sh -c 'nixos-rebuild switch'` is not matched. This is known: the module
   comment calls the list a guardrail. The pkexec dialog is what the user
   actually approves, so the skill already says "say what you are about to write
   before you ask". No change, but `docs/contract.md` will state it plainly.
5. **The VexPortal contract is broken in two ways, not one.** The six `just ai-*`
   recipes are missing (confirmed: none exist in the vexos-nix justfile). Also,
   VexPortal runs recipes in its own VTE terminal, which has both a TTY and
   `WAYLAND_DISPLAY`. So today `vexos-ai pick` would open zenity on top of
   VexPortal even with the recipes in place.
6. **Omarchy's OpenCode theming is weaker than the brief suggests.** It ships
   `"theme": "system"` and sends `SIGUSR2` to reload, so OpenCode follows the
   terminal palette. It writes no accent theme. A real accent theme from
   `~/.config/opencode/themes/vexos.json` would go further than Omarchy does.
7. **Skills drift: none today.** Every `just` recipe and `/etc/nixos` file the
   skills name exists in vexos-nix `040e63e`. The side-files come from
   `template/etc-nixos-flake.nix`. The test in Phase 2 keeps it that way.
8. **OpenCode already reads `~/.claude/skills`, and also `~/.agents/skills` and
   `~/.config/opencode/skills`.** Linking into `~/.agents/skills` adds nothing
   for OpenCode. It is the generic location other agents use, so we add it anyway
   at no cost. Omarchy does the same.

---

## 1. Gap table

| Area | Omarchy 4 | vexos-ai today | Proposed |
|---|---|---|---|
| Agents | 13 CLIs; default picked at setup | Claude Code, OpenCode | Same two (per brief) |
| Default choice | `omarchy default agent <name>`, menu, first-run toast | `pick`, zenity only, no arg | `pick <claude\|opencode>`; GUI only when no arg |
| Prompt entry | `omarchy agent prompt "<task>"` (auto-approve) | `--prompt "<text>"` (asks) | Keep `--prompt`; add `prompt <text…>` alias; always asks |
| Dedicated window | Super+Shift+Ctrl+A | Super+Shift+A, terminal via xdg-terminal-exec | Same key; own title, app-id where the terminal supports it |
| Accounts | add/use/list/remove/rename/mode auto [t] | add/use/list/remove/mode | + `rename`; non-interactive; refuse while a session uses it |
| Usage | 15 min refresh; plan; scoped model limits; last-known numbers kept; 429 Retry-After | 10 min timer, active account only; 5 min cache; `{ok:false}` on any error | Backoff + Retry-After, last-known kept with `stale`, plan label, scoped limits, single-flight lock |
| Token stats by day/model | Transcript scan (≈700 lines Python) | none | **Not now** (cost vs value); revisit after panel |
| Agents panel | Top-bar panel, left-click panel, right-click launch | zenity `panel` | GNOME Shell extension (Phase 5), reads `status --json` |
| Settings UI | Omarchy menu (walker) | zenity | **Decision A**: VexPortal deep link, or an app here |
| Notifications | `notify-send` + waiting process | `notify-send --wait` (action dies with the process, lost after reboot) | GNotification via D-Bus-activated app: actions work from the message tray |
| Crash capture | coredump watch, `crash <pid>`, mute, capture toggle | watch, `crash`, `crash-mute name [off]` | + `crash-mute --off`, `crash-capture on\|off`, unit gated on a flag |
| Skills | one OS skill linked into 6 agents' dirs, "experimental", "plan mode first" | 2 skills → `~/.claude/skills` | + `~/.agents/skills`; skill-drift test; plan-first line in the skill |
| Theme | Claude, OpenCode (`system`), Pi follow theme | Claude custom theme; OpenCode `system` once | + OpenCode `vexos` accent theme; reload running OpenCode |
| Safety | Auto-approve modes everywhere | Deny list; *but* Claude starts in auto, OpenCode allows all | Managed: no auto/bypass in Claude; `ask` in OpenCode; deny list unchanged |
| Search | none | none | GNOME Shell search provider: "Ask the assistant: …" |
| Machine contract | per-script `omarchy:args` metadata | file layout + interactive prompts | `status --json` v1, argv contract, exit codes, `docs/contract.md` |
| Tests | — | shellcheck | Contract tests (bats), unit tests, skill-drift fixture, NixOS VM test optional |

---

## 2. The contract (Phase 1)

### 2.1 Rules

- **Non-interactive by arguments.** Every mutating command takes its input as
  argv and never prompts when it has the argv it needs.
- **Interaction is a fallback only.** It happens when arguments are missing
  **and** a TTY or GUI is present. `--non-interactive` (or
  `VEXOS_AI_NONINTERACTIVE=1`) turns it off. Missing input then exits with 4.
- **Exceptions that are interactive by nature:** `account add` (browser sign-in
  via `claude auth login`) and launching the agent itself. Without a TTY these
  open a terminal window, as today. With `--non-interactive` they exit 4.
- **Exit codes:** `0` ok · `1` runtime failure · `2` usage error (bad argv or
  value) · `3` not found (no such account or program) · `4` needs interaction ·
  `5` busy (for example, account in use by a running session).
- **Output:** humans get one line on stdout and errors on stderr, prefixed
  `vexos-ai:`. Machines use `status --json`. Mutating commands accept `--json`
  and then print the new `status` document, so a caller can refresh in one call.
- **Files stay where they are and keep their formats.** New state is additive.
  The current VexPortal keeps working unchanged until it migrates.

### 2.2 argv (complete; existing forms are kept)

```
vexos-ai                                  open the assistant (in /etc/nixos)
vexos-ai --prompt "<text>"                … with a task           (existing)
vexos-ai prompt <text…>                   same, words joined      (new alias)
vexos-ai pick [claude|opencode]           set the agent; no arg → UI
vexos-ai status [--json] [--refresh]      state; --refresh re-probes usage (rate-limited)
vexos-ai account list
vexos-ai account add <label>              interactive sign-in (terminal)
vexos-ai account use <label|next>
vexos-ai account remove <label>
vexos-ai account rename <label> <new>     (new; not 'main')
vexos-ai account mode <manual|auto> [threshold]
vexos-ai crash-mute                       list
vexos-ai crash-mute <prog> [on|off]       (existing form)
vexos-ai crash-mute --off <prog>          (new form; same as '<prog> off')
vexos-ai crash-capture [on|off]           (new) no arg → print state
vexos-ai crash <pid> [comm exe sig]
vexos-ai diagnose [unit|rebuild]
vexos-ai usage [account]                  usage JSON (existing, kept)
vexos-ai theme
vexos-ai panel                            → settings UI (Decision A)
internal: crash-watch, usage-check, theme-watch, welcome   (replaced by `service` in Phase 3)
```

`pick` with no argument and no TTY or GUI exits 4. It never opens zenity.

### 2.3 `status --json`, schema 1

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
        "session": 12.0, "sessionResetsAt": "2026-10-06T21:00:00+00:00",
        "weekly": 40.0,  "weeklyResetsAt":  "2026-10-10T08:00:00+00:00",
        "scoped": [ { "label": "Fable Weekly", "percent": 30.0, "resetsAt": "…" } ],
        "fetchedAt": "2026-10-06T20:41:12+00:00",
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

- `schema` is an integer. It increases only on a breaking change. Additive keys do
  not bump it. Consumers must ignore unknown keys.
- `agent` and `usage` may be `null`. `usage: null` means no reading yet.
- Percentages are 0–100 floats, and the fraction/percent ambiguity is resolved
  here (as Omarchy does).
- `needsReboot` uses the same rule as VexPortal: in `/run/current-system`, not in
  `/run/booted-system`. `installed` is always true when the CLI runs. It is in
  the document so a caller with a cached copy can tell.
- `signedIn` is false only when the *refresh* token has lapsed. An expired access
  token is routine and keeps `signedIn: true`, with `why` = "start Claude Code to
  refresh it". Omarchy learned this one the hard way.

### 2.4 Files (unchanged; new ones marked ✚)

| Path | Format |
|---|---|
| `$XDG_CONFIG_HOME/vexos/ai/agent` | `claude`\|`opencode` |
| `…/claude-account`, `…/mode`, `…/threshold` | first line |
| `…/crash-ignore/<prog>` | empty file per muted program |
| ✚ `…/crash-capture-off` | flag file. The unit's `ConditionPathExists=!` reads it at the default `~/.config` path |
| `$XDG_DATA_HOME/vexos/ai/claude-accounts/<label>/` | Claude config home |
| `$XDG_CACHE_HOME/vexos/ai/usage-<label>.json` | existing keys `ok session weekly sessionReset weeklyReset why` kept. ✚ additive: `fetchedAt plan scoped stale` |
| ✚ `$XDG_STATE_HOME/vexos/ai/probe-<label>.json` | backoff state (`nextAt`, `failures`). Never holds a token |

### 2.5 Usage endpoint hardening

- The token is read from `.credentials.json`, used for one `Authorization`
  header, and dropped. It is never in argv, logs, cache, state, error text or
  `status`. In Rust it is a `Secret` type whose `Debug` impl prints `[redacted]`.
  A test fails if the token string appears in any written file or in output.
- **Single-flight:** an `flock` on `probe-<label>.lock`, so the timer, VexPortal
  and the panel never probe the same account at once.
- **Intervals:** active account every 10 min. Every 3 min when within 15 points of
  the threshold. Inactive accounts every 60 min. `--refresh` skips the interval
  but not backoff, with a 15 s floor.
- **Backoff:** on 429, honour `Retry-After`. Otherwise use exponential backoff:
  1, 2, 4 … 60 min. A transport error (no route yet after login) retries after
  1 min without counting as a failure.
- **Degrade:** keep the last good numbers with `stale: true`. Reset a window whose
  `resetsAt` has passed to 0. `why` is a short human sentence. An unknown payload
  shape gives `ok: false, why: "usage format changed"` and is never a crash.

---

## 3. GNOME surfaces, in priority order

1. **Notifications with real actions (GNotification).** *Build.* Today
   `notify-send --wait` keeps a process alive per notification, and the action
   dies with it. That breaks after a logout, or when the user clicks from the
   message tray later. A D-Bus-activated app (`DBusActivatable=true`, the
   prerequisite for persistent notifications) receives
   `app.diagnose-crash(pid)`, `app.switch-account(label)`, `app.setup` and
   `app.diagnose-rebuild` even when it was not running. Covers crashes, usage
   near a limit (button: "Switch to <best>"), the welcome prompt, and rebuild
   failures (vexos-nix calls `vexos-ai notify rebuild-failed` from
   `just rebuild`). Works on GNOME. On Hyprland and COSMIC it works through the
   freedesktop fallback, minus persistence.
2. **Settings UI: first-run picker, accounts, usage.** *Decision A.*
   Recommended: **(a)** deep-link VexPortal's AI page. It already renders exactly
   this state, and a second UI would fork it. What that needs:
   - VexPortal handles `vexportal --page ai` (`HANDLES_COMMAND_LINE`) or a
     `app.show-page('ai')` action. It has neither today.
   - VexPortal calls `vexos-ai … --json` directly for user-scope state, instead of
     a terminal dialog per click. The terminal stays for `account add`.
   - Fallback when VexPortal is absent (Hyprland/COSMIC minimal installs, or
     VexPortal off): the picker becomes a TTY prompt in the agent's own terminal
     window, and `panel` prints `account list`. No GTK in this repo.

   Option **(b)**, a GTK4/libadwaita app here, gives the same UI everywhere but
   duplicates VexPortal's page. Pick it only if vexos-ai must stand alone outside
   VexOS.
3. **Desktop entry, app-id, icon, window.** *Build.* App-id
   `io.github.victorytek.VexosAssistant`. The desktop file is named after it, is
   `DBusActivatable`, and has a symbolic icon shipped in `hicolor/symbolic`.
   Actions: Open, Accounts & usage, Diagnose a problem, Change AI tool. The agent
   terminal uses `xdg-terminal-exec --app-id=… --title="VexOS Assistant"
   --dir=/etc/nixos`. Honest caveat: GNOME Console declares no
   `X-TerminalArg*` keys, so it ignores app-id and title. The window then groups
   under Console. Ptyxis, foot, kitty and ghostty honour them. A desktop-file
   rename means vexos-nix updates `favorite-apps` if it pins the old id. It does
   not today.
4. **Search provider.** *Build.* `org.gnome.Shell.SearchProvider2` on the same
   D-Bus app. It returns one result, "Ask the assistant: <query>", for queries of
   two or more words, so typing `firefox` never shows it. `ActivateResult` runs
   `vexos-ai --prompt "<query>"`. That is argv, never a shell string, in
   `/etc/nixos`, in the default asking mode. The user can turn it off in
   Settings → Search like any provider.
5. **Top-bar usage indicator (GNOME Shell extension).** *Build last; Decision D.*
   A small ESM extension (`vexos-ai@victorytek`, `shell-version: ["50"]`):
   - an icon with the active account's highest % in the top bar
   - left-click: a menu with per-account session and weekly bars, reset times,
     plan, "Use", and auto-switch
   - right-click: launch the assistant

   It reads `status --json` through `Gio.Subprocess` on a 60 s timer, plus a
   `Gio.FileMonitor` on the cache dir. It never touches tokens or the network.
   The flake ships it in `share/gnome-shell/extensions/`, and vexos-nix adds the
   UUID to `vexos.gnome.extraExtensions` when the ai feature is on. Cost: a
   `shell-version` bump and smoke test on every GNOME major (6-monthly). vexos-nix
   already carries 11 extensions under the same rule. GNOME 50 has no breaking
   changes for panel indicators.
6. **Theme sync for OpenCode.** *Build.* Write `~/.config/opencode/themes/vexos.json`
   from the accent and scheme, using the documented `defs` + dark/light format.
   Set `tui.json` `theme: "vexos"` only when the theme is unset, `system`, or
   already ours (the same rule as for Claude). Reload running instances with
   `SIGUSR2` (Omarchy does this). Gio watches `org.gnome.desktop.interface`
   in-process, replacing `gsettings monitor`.

---

## 4. Rust or bash

**Recommendation: Rust, with the CLI argv frozen by black-box contract tests
written first in Phase 1.**

Why Rust:

- Surfaces 1, 3 and 4 need a long-lived GApplication that owns a D-Bus name,
  exports actions and a search-provider object, and is D-Bus activatable. bash
  cannot host D-Bus objects. Without Rust we would need bash plus a second
  program anyway.
- `gio` + `glib` without GTK, as long as Decision A is (a), gives notifications,
  D-Bus, GSettings watching and file monitors in one small binary. It is the same
  stack as VexPortal and vex-vpn.
- The token stays in process memory (no curl, no jq). Typed JSON for the
  endpoint and for `status`. Unit tests for parsing, backoff, threshold and
  account logic, which are hard to test in bash.
- A `vexos-ai-core` library crate holds the `status` types. VexPortal *can*
  depend on it as a git dependency, but does not have to: the JSON contract is
  the boundary.

Cost and risk: a rewrite of about 580 lines, plus a crate build in `nix build`
(`rustPlatform.buildRustPackage`, `cargoLock`). Mitigation: the bats contract
suite from Phase 1 runs unchanged against both implementations. The port is done
when it passes.

Why not stay in hardened bash: it stays viable for the CLI alone. But every GNOME
item above would then need a second language, and the shared state logic would
live in two places.

Phase 1 can ship either way:

- **1-bash**: extend the current script (small, fast, unblocks VexPortal this
  week). Rust replaces it in Phase 2.
- **1-rust**: port first, then add the contract. Slower to unblock, no throwaway.

Recommended: **1-bash**. The throwaway is about 150 lines. The contract tests are
the durable part.

---

## 5. Phases (each ships on its own)

**Phase 0: Safety fixes (module only, small). Done.**
- `nix/policy.nix`: the Claude drop-in adds `permissions.disableAutoMode` and
  `permissions.disableBypassPermissionsMode`, both `"disable"`, with the deny list
  unchanged. OpenCode gets `ask` + denies, global and per built-in agent (see
  §0.2).
- `checks.policy` renders both JSON files and asserts every key, including
  `"*"` being the first bash rule. Negative-tested.
- Skill: rule 6, "Plan first".
- Validated: `nix build`, `nix flake check`, the vexos-nix eval with
  `--override-input`, and opencode 1.18.34 `debug agent` against the rendered
  config.

**Phase 1: Contract (unblocks VexPortal).**
- argv per §2.2 (`pick <agent>`, `account rename`, `crash-mute --off`,
  `crash-capture`, `prompt`, `--non-interactive`, `--json`), exit codes, no zenity
  when argv is given.
- `status --json` schema 1. The usage hardening in §2.5 that bash can do: backoff
  state, Retry-After, last-known kept, flock.
- `crash-capture`: flag file + `ConditionPathExists=!` on the crash-watch unit,
  plus `systemctl --user start|stop`.
- Skills also linked into `~/.agents/skills`.
- `docs/contract.md`. `tests/contract/*.bats` (black-box, fake `HOME`, stub
  `claude`, `curl`, `systemctl`, `notify-send` on PATH), wired into
  `checks.<system>.contract`.

**Phase 2: Rust core + skill-drift test.**
- Workspace: `crates/vexos-ai-core` (state, accounts, usage, status types),
  `crates/vexos-ai` (CLI). Same argv. bats suite passes. `cargo test` in `checks`.
- Skill-drift: `tests/fixtures/vexos-nix.json` = `{rev, recipes: {name: [params]},
  sideFiles: [...]}`, generated by `scripts/update-fixture <vexos-nix checkout>`
  from `just --dump --dump-format json` and `template/etc-nixos-flake.nix`. A
  check greps `just <recipe>` and `/etc/nixos/<file>` and backticked `*.nix`
  names in `skills/` and fails on any name missing from the fixture. This is the
  same fixture pattern VexPortal uses. It needs no vexos-nix flake input, so no
  cycle and no extra fetch for consumers. The fixture's `rev` is compared to
  vexos-nix's lock in the vexos-nix hand-off.

**Phase 3: The GApplication.**
- `vexos-ai service` replaces the four user units with one: crash watch, theme
  watch, usage polling, welcome, all in-process. D-Bus activatable, app-id per §3.
- GNotification actions. `vexos-ai notify rebuild-failed`.
- Desktop file, symbolic icon, terminal app-id and title.
- Drop `zenity` and `libnotify` from runtime inputs once nothing calls them.

**Phase 4: Search provider + OpenCode theme.**

**Phase 5: Top-bar extension** (if Decision D is yes).

Not planned: more agents, auto-approve, mise, a transcript token scanner.

---

## 6. Hand-off summary (details at the end of each phase)

**vexos-nix**, in order: push vexos-ai → `nix flake update vexos-ai` → dry-build.
- Phase 1: the six recipes as thin wrappers, e.g.
  `ai-pick agent="": vexos-ai pick {{agent}}`,
  `ai-crash-mute name="" state="": vexos-ai crash-mute {{name}} {{state}}`
  (the VexPortal signatures, unchanged), each behind the existing
  "not installed" guard.
- Phase 3: `just rebuild` failure branch calls `vexos-ai notify rebuild-failed`.
  The dconf keybinding command stays `vexos-ai`.
- Phase 5: add the extension UUID to `extraExtensions` in `modules/ai.nix`
  (desktop GNOME only).

**VexPortal**
- Replace `AiState::read()` with `vexos-ai status --json` (schema 1). Keep the file
  reader as a fallback while `schema` is missing.
- Fix the module comment that points at `pkgs/vexos-ai/vexos-ai.sh` (now
  `github:VictoryTek/vexos-ai` `bin/vexos-ai.sh`, then the Rust CLI).
- Decision A(a): add `--page ai` / `app.show-page`, and run non-interactive
  `vexos-ai` commands directly.

---

## Decisions

| | Question | Decided |
|---|---|---|
| A | Settings UI: VexPortal deep link + TTY fallback, or a GTK app here | **(a) VexPortal** |
| B | Rust rewrite, and whether Phase 1 is bash first | **Rust; Phase 1 in bash with bats contract tests** |
| C | Phase 0: make both agents actually ask (Claude: no auto/bypass; OpenCode: `ask` for edit and bash) | **Yes.** It is what "VexOS agents ask" already claims |
| D | Top-bar GNOME Shell extension | **Yes, last phase** |
