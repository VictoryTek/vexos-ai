# Assistant panel app, first slice — spec

Status: Phase 1, 2026-10-25. Builds on `assistant_panel_spec.md` (step 3 of its
section 7) and the core in `crates/vexos-ai-core`. No GNOME extension work.

## 1. Request

0. Live plan-exit re-check (Claude, plan allowed) under the managed policy,
   recorded in `assistant_panel_spike_results.md` under "Plan-exit live check".
1. A view-model layer with no GTK dependency, unit-tested and mutation-checked.
2. A minimal GTK4/libadwaita window on it, packaged, with a "Run `just rebuild`"
   button.

Out of scope: `nix/policy.nix`, `docs/contract.md`, the CI workflow, the core's
safety behaviour, the Shell extension, D-Bus activation, installing the app
from `nix/module.nix`.

## 2. Current state

- The core's public API: `Session::{spawn, connect, prompt, decide, cancel,
  agent}`, `Event::{AgentText, ToolCall, Permission, Aborted, Closed}`.
  `PermissionRequest` carries `title`, `kind`, `raw_input` and `choices` (a
  subset of `[ApproveOnce, DenyOnce]`). There is **no mode in the API**.
- `raw_input` is enough to show what a request would do (live captures):
  Claude `Edit` `{file_path, old_string, new_string}`, `Write`
  `{file_path, content}`, `Bash` `{command}`, `ExitPlanMode` `{plan,
  planFilePath}`; OpenCode `bash` `{command}`, `edit` `{filepath, diff}` (a
  unified diff, `out/smoke/p0c-opencode-edit`).
- Many `tool_call_update`s carry no title (smoke-test finding 5).
- `checks.core` builds the whole workspace with `buildRustPackage`.

## 3. Decisions taken with the user (2026-10-25)

- **Mode display needs a core change.** Add `Event::Mode(String)`, emitted
  **only from state the core has confirmed**: after the initial pin, after a
  reset that was acknowledged and is still the newest, and for an allowed mode
  report while no reset is running. Never for a disallowed or unreadable report,
  never from an older reset. Pinning, the allow-list, the options filter and
  deny-by-default do not change. Own test, mutation-checked; listed separately.
- **Rebuild privilege:** an embedded VTE terminal (`vte4`). `just rebuild` calls
  `sudo`, which needs a pty to ask for the password.
- **Rebuild isolation:** the fixed argv `["just", "rebuild"]` in `/etc/nixos`,
  spawned directly (no shell, so nothing to type into once it exits; input is
  disabled on exit). The argv is a constant, never built from agent output or
  other text. Only after a click and a confirmation dialog. The terminal's
  output never reaches the conversation, and nothing in the agent path can
  start it or write to it.

## 4. Architecture

```
crates/vexos-ai-core       (+ Event::Mode)
crates/vexos-ai-viewmodel  no GTK: Conversation, ApprovalCard, Proposal, diff,
                           Rebuild state machine; depends on the core only
crates/vexos-ai-panel      GTK4 + libadwaita + vte4 binary on the view-model
```

### 4.1 View-model rules (each one tested)

| Rule | How |
|---|---|
| Fed by the core's `Events` | `Conversation::apply(Event)` |
| Tool calls merged by id | a later `None` field keeps the earlier value |
| Only offered options | a card's buttons are its `choices`, labelled "Approve once"/"Deny once"; nothing else exists |
| Mode read-only | `mode()` is set only by `Event::Mode`; no setter, no picker |
| Show the change | `Proposal::{Command, Diff, Plan, Other}` from `raw_input`; edits give a line diff |
| Answer only on a click | `click()` is the only call to `Decider::decide`; `apply` never decides |
| Close denies | `close()` sends `DenyOnce` for every pending card that offers it; the app then drops the session, which answers the rest |
| No branding as "Claude Code" | `agent_label` |

`Decider` is a one-method trait (`decide`) implemented for `Session`, so tests
record calls with a fake.

`Rebuild`: `Idle → Confirming → Running → Finished`. `confirm()` returns the
constant `RebuildCommand` only from `Confirming`; a second run cannot start
while one is running. `Conversation` holds no rebuild state.

### 4.2 Window

`adw::ApplicationWindow`, app-id `io.github.victorytek.VexosAssistant`, plain
and centred. Header: title, subtitle "<agent> · <mode>", Cancel, "Run just
rebuild". First page: agent choice (Claude / OpenCode); once chosen it is gone.
Then: message list, approval cards, input. The core runs on a tokio runtime;
events are read on the GLib main loop (`Events::recv` needs no reactor).
Closing the window calls `close()`, then drops the session inside the runtime.

Rebuild: confirmation `adw::AlertDialog`, then a separate window with a
`vte4::Terminal`, `spawn_async(argv, "/etc/nixos", SEARCH_PATH)`. On
`child-exited` the input is disabled and the exit status shown. That window
can't be closed while the command runs.

## 5. Dependencies (versions resolved together in a scratch project)

`gtk4` 0.11.5, `libadwaita` 0.9.2, `vte4` 0.10.0 (no duplicate `glib`/`gio`).
nixpkgs (lock `nixos-26.05`): gtk4 4.22.4, libadwaita 1.9.3, vte-gtk4 0.84.1,
rustc 1.95. Context7: gtk4-rs (tokio with the GLib loop), libadwaita
(`AlertDialog`). VTE has no Context7 entry; the `vte4` 0.10.0 source was read
(`spawn_async`, `set_input_enabled`, `connect_child_exited`).

## 6. Nix

- `packages.<system>.vexos-ai-panel` (`nix/panel.nix`): `buildRustPackage`,
  `-p vexos-ai-panel`, `wrapGAppsHook4`, desktop entry. Not installed by the
  module yet.
- `checks.core` builds and tests `-p vexos-ai-core -p vexos-ai-viewmodel` (no
  GTK), plus clippy for both.
- `checks.panel`: the panel package plus clippy. It compiles the GTK code and
  runs nothing that needs a display.
- devShell: `pkg-config`, `gtk4`, `libadwaita`, `vte-gtk4`.
- `scripts/preflight.sh` (new; Phase 6): `nix build`, `checks.{policy,contract,
  core,panel}` and the module `nix eval`. Never `nix flake check`.

## 7. Verification

`cargo build --all-targets`, `cargo test`, `cargo clippy --all-targets -- -D
warnings`, `cargo fmt --check` (in the devShell), `nix build`,
`checks.{policy,contract,core,panel}`, preflight; mutation checks for the core
mode event and the view-model rules; one launch against a real agent inside
`spike/acp/sandbox.sh` if a display is available.

## 8. Risks

| Risk | Mitigation |
|---|---|
| A mode shown that the core hasn't confirmed | Emit rules above; test with mutations |
| The UI approving without a click | `apply` has no `Decider`; tests assert no calls |
| Rebuild driven by the agent | Constant argv; separate window; no API from the conversation |
| The user closes a running rebuild | Close refused until the child exits |
| `Session` dropped off the runtime skips its async abort | Drop inside `runtime.enter()` |
| `claude-agent-acp`/`just` not on PATH | Error shown in the window; the module option comes later |
