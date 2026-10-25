# Assistant panel: drop-down chat and Quick Settings tile — spec

Status: **Phase 1 (research and specification)**, 2026-10-24. No code changed.
Mockup (approved by the user): https://claude.ai/artifact/7kNYWvE47wDGZkQp8uuj9G
Builds on `docs/spec.md` (decided 2026-10-06). Phases 0 and 1 of that plan are
done; its Phase 2 (Rust core) is next and is a prerequisite here.

## 1. Request

A native, polished UI for talking to the AI assistant about OS tasks, in place
of a TUI in a terminal:

- a **chat panel** that drops down from the top bar on a shortcut, Quake-style;
- a **GNOME Quick Settings tile** (toggle, agent and account menu, usage,
  crash and rebuild alerts);
- decisions already taken with the user: native chat UI (not a terminal);
  GTK4 app plus a thin Shell extension; all four tile functions.

## 2. Current state

- The assistant is a bash CLI (`bin/vexos-ai.sh`) that opens Claude Code or
  OpenCode in a terminal via `xdg-terminal-exec` (`in_new_terminal`,
  `bin/vexos-ai.sh:80`). `nix/module.nix` installs both agents, the skills, the
  managed safety policy and four user services.
- `docs/spec.md` already plans: a Rust rewrite (Phase 2), a D-Bus-activated
  GApplication that owns notifications (Phase 3), a search provider (Phase 4)
  and a top-bar extension (Phase 5). It decided **A**: settings UI is a VexPortal
  deep link, **no GTK in this repo**; and **D**: extension last.
- vexos-nix binds a dconf custom keybinding that runs `vexos-ai`
  (`modules/gnome-desktop.nix:55-66`) and appends extension UUIDs through
  `vexos.gnome.extraExtensions` (`modules/gnome.nix:40`).
- The safety policy (`nix/policy.nix`, asserted by `checks.policy`) denies
  applying a configuration and keeps both agents asking.

## 3. Problem

1. There is no chat UI: a TUI cannot show approvals, diffs or usage the way the
   mockup does, and cannot be a Quake-style surface.
2. Driving an agent headlessly means re-implementing its permission prompts in
   the UI. That is the one place the "never apply, always ask" rule can break.
3. Two agents (Claude Code, OpenCode) with two native protocols would mean two
   backends.

## 4. Research findings

Sources (all read in this phase):

| # | Source | Finding used |
|---|---|---|
| 1 | ACP, Tool Calls (agentclientprotocol.com/protocol/tool-calls) | `session/request_permission` takes `sessionId`, `toolCall`, `options[]` with kinds `allow_once`, `allow_always`, `reject_once`, `reject_always`. The client answers `selected{optionId}` or `cancelled`. Tool-call content can be `diff` (path, oldText, newText), `terminal` or text. Statuses: pending, in_progress, completed, failed. |
| 2 | ACP Rust SDK via Context7 `/agentclientprotocol/rust-sdk` (v3.5.0) | `Client.builder().on_receive_notification(SessionNotification)` and `.on_receive_request(RequestPermissionRequest)`; `InitializeRequest`, `NewSessionRequest`, `PromptRequest`; feature `process` provides `AcpAgent` to launch a child agent. |
| 3 | opencode.ai/docs/acp | `opencode acp` runs OpenCode as an ACP agent over stdio. Built-in tools, MCP, `AGENTS.md`, **agents and the permissions system** work as in the terminal. `/undo` and `/redo` unsupported. |
| 4 | github.com/zed-industries/claude-agent-acp (Apache-2.0) | Adapter that exposes Claude Code (Claude Agent SDK) as an ACP agent: tool calls, diffs, edit review, terminals. `CLAUDE_AGENT_ACP_PROTOCOL=v1` pins the stable protocol; the v2 surface is a draft. nixpkgs (unstable index) has `claude-agent-acp` 0.21.0. |
| 5 | code.claude.com/docs/en/agent-sdk/claude-code-features | **Managed policy settings are read regardless of `settingSources`.** Skills load from `~/.claude/skills` like the CLI. So `/etc/claude-code/managed-settings.d/50-vexos.json` still applies under the SDK. |
| 6 | code.claude.com/docs/en/agent-sdk/permissions | Order: hooks, **deny rules (blocked even in `bypassPermissions`)**, ask rules, mode, allow rules, `canUseTool`. From Claude Code 2.1.302 an SDK app starts in `default`; unapproved calls are denied unless something approves them. |
| 7 | gjs.guide/extensions/topics/quick-settings | `QuickSettings.SystemIndicator` + `QuickMenuToggle` (`title`, `subtitle`, `iconName`, `menu.setHeader`, sections), registered with `Main.panel.statusArea.quickSettings.addExternalIndicator`, destroyed in `disable()`. |
| 8 | gjs.guide/extensions/overview/anatomy | `metadata.json` (`uuid`, `shell-version` major only); `disable()` must undo `enable()`: the most common review rejection. |
| 9 | ddterm docs/CommandLine.md and extensions.gnome.org | Precedent for exactly this split: a GTK app (`com.github.amezin.ddterm`) controlled with `gapplication action … show/hide/toggle`, plus a Shell extension that also exposes `Toggle`/`Activate` over D-Bus. Works on Wayland. Extension needed fixes for GNOME 49. |
| 10 | mutter issues #1533, #2407; GNOME Discourse segfault thread | On Wayland, `get_frame_rect` right after `move_resize_frame` returns stale values; tiled windows reject moves; a tiling extension segfaulted in `move_resize_frame`. |
| 11 | gtk4-rs book via Context7 `/gtk-rs/gtk4-rs` | `DBusActivatable=true` plus a D-Bus service file: raises the existing window instead of a second instance, and is a prerequisite for persistent notifications. Actions via `gio::ActionEntry`. |
| 12 | Claude Code headless docs; opencode.ai/docs/server | The alternative route: `claude -p --output-format stream-json --permission-prompt-tool <mcp tool>` and `opencode serve` (HTTP/OpenAPI). Two protocols, two permission mechanisms. |

Not verified (must be settled by the Phase 2a spike, section 7):

- whether `claude-agent-acp` honours `CLAUDE_CONFIG_DIR`, which the multi-account
  feature relies on (`$XDG_DATA_HOME/vexos/ai/claude-accounts/<label>/`);
- whether the adapter or OpenCode-over-ACP ever starts in an auto-approving mode;
- the nixpkgs 26.05 versions (the nixos MCP index only knows unstable and 25.11:
  it lists opencode 1.4.0, which disagrees with the 1.18.34 recorded in
  `docs/spec.md`, so its package data is stale and not relied on here);
- Anthropic's current terms for driving a subscription login through the Agent
  SDK from a third-party-style UI. This is personal local tooling, but check
  before shipping.

## 5. Proposed architecture

```
 shortcut / tile ──► gapplication action toggle ──► vexos-ai-panel (GTK4/libadwaita, D-Bus activatable)
                                                      │  ACP client (stdio, JSON-RPC, protocol v1)
                                                      ├──► claude-agent-acp ──► Claude Code (managed policy applies)
                                                      └──► opencode acp     ──► OpenCode   (managed policy applies)
 Shell extension vexos-ai@victorytek:
   - Quick Settings tile (toggle, menu, usage, alerts)  ◄── `vexos-ai status --json` (60 s + file monitor)
   - slides the panel window down from the top bar
```

### 5.1 One protocol: ACP

The panel is an ACP **client** (crate `agent-client-protocol` 3.x, `process`
feature). It spawns the chosen agent as a child process, so Claude Code and
OpenCode share one code path for streaming messages, tool calls, diffs and
permission requests. It advertises **no** `fs` or `terminal` client capability,
so the agent uses its own tools, exactly as in a terminal session.

Alternative rejected (unless the spike fails): stream-json plus
`--permission-prompt-tool` for Claude and `opencode serve` for OpenCode. It
costs two backends, two permission mechanisms and two event models.

### 5.2 Safety rule, unchanged and enforced in the UI

- `nix/policy.nix` is **not changed**. Deny rules and `disableAutoMode` come from
  managed settings that the agent process loads regardless of how it is driven
  (source 5, 6).
- The panel answers `session/request_permission` only from a user click on a
  card. It shows **Approve** (`allow_once`) and **Deny** (`reject_once`). It
  never auto-selects, never offers `allow_always`/`reject_always`, and answers
  `cancelled` when the turn is cancelled or the window closes.
- The panel never sets a permission mode (no auto, no bypass, no accept-edits).
- Applying stays with the user: the panel shows "Run `just rebuild`" as a
  **user-clicked** button that opens a terminal; the agent cannot trigger it.
- New checks (section 8): a unit test with a fake ACP agent proves the above;
  `checks.policy` continues to assert the managed files.

### 5.3 Components

| Component | Where | What |
|---|---|---|
| `crates/vexos-ai-core` | this repo (`docs/spec.md` Phase 2) | status types, accounts, usage. Shared. |
| `crates/vexos-ai-panel` | this repo | GTK4 + libadwaita app. App-id `io.github.victorytek.VexosAssistant`, `DBusActivatable=true`. Actions: `toggle`, `show`, `hide`, `diagnose-crash(pid)`, `diagnose-rebuild`, `prompt(text)`. Holds the ACP session, renders the transcript (markdown via `pulldown-cmark` to text tags; code blocks, diffs, tool cards), the approval cards, the composer, history. Falls back to a normal centred window when the extension is absent (also Hyprland/COSMIC). |
| `vexos-ai toggle` (and `vexos-ai prompt` opens the panel when it can) | CLI | thin wrapper over `gapplication action`. Additive argv, so `docs/contract.md` and the bats suite are updated together. |
| `extension/vexos-ai@victorytek/` | this repo, shipped in `share/gnome-shell/extensions/` | `metadata.json` (`shell-version: ["50"]`), `extension.js` (indicator, tile, window placement), no tokens, no network. |
| `nix/module.nix`, `nix/package.nix` | this repo | build the crates, install the desktop file, D-Bus service file, icon and extension; new package options for the ACP adapter. |

### 5.4 Window placement (the risky part)

The panel is a normal `AdwApplicationWindow` (undecorated, fixed width, rounded
bottom corners via CSS). The extension matches it by app-id on `window-created`
and, once, calls `move_resize_frame` to the top-centre under the panel bar,
sets above and sticky, and animates the **window actor's** translation/opacity
(compositor side) instead of stepping `move_resize_frame` per frame. It never
reads the frame rect back immediately (source 10). Toggle goes through the
app's `toggle` action, so a cold start works through D-Bus activation.
Everything the extension does is undone in `disable()`.

### 5.5 Tile (per the mockup)

`QuickMenuToggle`: title "Assistant", subtitle "<agent> · <N>% used",
`checked` while the panel is visible. Menu: agent radio items (runs
`vexos-ai pick <agent>`), account rows with session and weekly bars and "Use"
(`vexos-ai account use <label>`), "Needs attention" rows (Diagnose →
`diagnose-crash` / `diagnose-rebuild` action), "Open assistant". Data from
`vexos-ai status --json` (schema 1, already exists) every 60 s plus a
`Gio.FileMonitor` on the usage cache. All commands go through `Gio.Subprocess`
with argv arrays.

## 6. Changes to existing decisions in `docs/spec.md`

- **A** (VexPortal deep link, no GTK here) is **superseded for chat**: this repo
  now has a GTK4 app. Account and usage management inside the panel reuses
  `status --json`; VexPortal's AI page is unaffected and keeps working.
- **D** (extension last) stands as an ordering, but the extension now also
  places the panel.
- The planned Phase 3 GApplication and Phase 4 search provider become part of
  `vexos-ai-panel`'s process (same app-id), not a separate daemon.

## 7. Implementation steps (each ships on its own)

| Step | Work | Verify |
|---|---|---|
| 2 (existing) | Rust core, CLI port, skill-drift test, per `docs/spec.md`. **Prerequisite.** | bats `contract` check passes unchanged against the Rust CLI |
| 2a | **Spike, no shipping:** run `claude-agent-acp` and `opencode acp` with a throwaway ACP client under the real managed settings and `CLAUDE_CONFIG_DIR`. | (a) `nixos-rebuild switch` is denied; (b) an unlisted command arrives as `request_permission`; (c) no auto-approval; (d) account dir honoured. **If (a)-(c) fail, stop and report; fall back to the stream-json route.** |
| 3 | `vexos-ai-panel`: window, ACP client, transcript, approval cards, composer, usage header, desktop file, D-Bus service, `toggle` argv. | Rust unit tests with a fake ACP agent (never auto-approves, only Approve/Deny, `cancelled` on close); `nix build`; `contract` check; `policy` check |
| 4 | Extension: tile, menu, placement. | `gnome-shell --devkit`/nested session smoke test on GNOME 50 by the user; `eslint`-clean; `disable()` leaves no actors or signals |
| 5 | GNotification actions (crash, usage near limit, welcome, rebuild failed), search provider "Ask the assistant". | Per `docs/spec.md` section 3 |

Hand-off to vexos-nix after steps 3 and 4: dconf binding runs `vexos-ai toggle`;
`modules/ai.nix` adds `vexos-ai@victorytek` to `extraExtensions` (GNOME
desktop only); lock bump in both repos (same `nixos-26.05` rev).

## 8. Dependencies and configuration

- Rust: `agent-client-protocol` 3.x (`process`), `gtk4`, `libadwaita`, `glib`,
  `gio`, `tokio`, `serde`/`serde_json`, `pulldown-cmark`. Context7 verified now:
  ACP Rust SDK v3.5.0 API and gtk4-rs D-Bus activation. **To verify in Phase 2
  before adding:** exact `gtk4`/`libadwaita` versions that match the locked
  nixpkgs 26.05 (sibling repos use 0.7 to 0.9 lines), and `pulldown-cmark`.
- Nix: `claude-agent-acp` (new `programs.vexos-ai.claudeAcpPackage` option,
  default `pkgs.claude-agent-acp`, vexos-nix passes unstable), `opencode`
  (already). Rust build via `rustPlatform.buildRustPackage` with `cargoLock`.
- Env for the adapter: `CLAUDE_AGENT_ACP_PROTOCOL=v1`.
- No change to `flake.lock` in this phase. `nix flake check` is never used.
- Safe commands for later phases: `nix build`, the `policy` and `contract`
  single-check builds, and the module `nix eval`. No preflight script exists yet;
  Phase 6 Step 3 will create `scripts/preflight.sh` from these.

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Adapter or ACP mode weakens "always ask" | Spike 2a is a hard gate; unit tests with a fake agent; panel never sets a mode; managed deny list applies independently |
| Adapter ignores `CLAUDE_CONFIG_DIR` (breaks multi-account) | Spike 2a (d); otherwise single-account in the panel and say so |
| Pre-1.0 adapter and a draft ACP v2 | Pin v1; pin the package; contract tests on the panel's use of ACP |
| Wayland placement bugs (stale rects, tiled windows, crashes in the Shell) | One move, compositor-side animation, no readback, untile first; panel still works as a plain window if placement fails |
| Extension breaks each GNOME major | Same rule as vexos-nix's 11 other extensions: bump `shell-version` and smoke test |
| Markdown and diff rendering without WebKit | `pulldown-cmark` to text tags; plain monospace diffs; no web view |
| Subscription terms for SDK-driven login | Check Anthropic's current policy before shipping; OpenCode path unaffected |
| `sudo` or `pkexec` prompted by the agent | Unchanged: the user answers polkit; there is no TTY for `sudo`, so it fails safe |

## 10. Assumptions to confirm (non-blocking)

1. One continuous conversation per project directory (`/etc/nixos`), with a
   history list, not many parallel tabs.
2. Tile subtitle shows the active account's highest usage percentage.
3. The panel needs the extension only for the drop-down behaviour; without it,
   it is a centred window.
4. Switching agent or account starts a new session.
