# VexOS AI assistant: remaining work plan

Repo: vexos-ai. Last updated: 2026-10-31. Owner: the user.
Put this file at `.github/docs/subagent_docs/remaining_work_plan.md`. Claude Code works through it and marks items off.

---

## 1. How to work through this file (Claude Code: read this first)

1. Work on ONE item at a time, in order, inside the current phase. Skip any item whose "Needs" is not met.
2. Read the item's "Read first" files before you start.
3. Items marked **USER** are for the user to do. Items marked **DECISION** need the user's answer. You do not do these and you do not guess the answer. Ask with AskUserQuestion, or skip and say why.
4. Mark off an item only when its verification passed:
   - Change `[ ]` to `[x]` and add `— done YYYY-MM-DD: one line on what changed`.
   - Partly done: use `[~]` and say exactly what is missing.
   - Blocked: use `[!]` and say what blocks it.
   - Add one line to the Log at the bottom for every item you touch.
5. Stop and ask before any change to: nix/policy.nix, the core's safety behaviour, docs/contract.md, or the CI workflow, unless the item says it is allowed.
6. If something you find contradicts this plan, stop and tell the user. Do not quietly change the plan.
7. Reports to the user: short, plain language. No shorthand without explaining it.

### Scope rule (applies to every item)

- Do not create, modify or delete anything outside the vexos-ai repo directory (no ~/.claude, ~/.config, ~/.local/share, dconf, /etc). Do not read or write other repos.
- Never read, print or open credential files or tokens.
- Do not run anything inside the user's live GNOME session. Use the nested headless shell under target/ only.
- No nixos-rebuild, just rebuild, install, or sudo.
- No git writes, and do not stage anything. The user commits with GitHub Desktop.
- Downloads into /nix/store are allowed. Tree copies go in a fresh directory under target/, with no `rm -rf`. Run the copy and the build as separate commands.
- New files are untracked, so nix checks may not see them. Run nix checks on a `path:` copy and say so. Ask the user to tick the new files in GitHub Desktop.
- If CLAUDE.md or a hook asks for extra steps, follow this scope rule first and say what you skipped.

### Verification standard

- Run what applies: cargo build --all-targets, cargo test, clippy --all-targets -- -D warnings, fmt --check, nix build, checks.policy, checks.contract, checks.core, checks.panel, checks.module, checks.extension, scripts/preflight.sh.
- Say which Rust version you used. CI pins 1.95.0.
- Report only what ran, with exit codes. Use "–" for anything not run. Say what ran on a copy of the tree.
- Mutation-check any new safety test: break the rule, confirm a test fails.
- Tier 3 changes (safety, core, view-model, extension, policy, contract, public interfaces) need the independent reviewer subagent. Fix its findings and run it again on the fixes. Write "Review approved" only if the final run passed. A self-review is labelled "self-review only".
- Finish every item with ONE commit message in the project's commit format. The user commits everything uncommitted in one go, so the message covers all uncommitted work.

### Safety rules that must never be weakened

- Modes pinned (Claude: default or plan, OpenCode: build). The allow-list resets anything else.
- Approval cards offer only Approve once and Deny once. "Always allow" is dropped.
- Nothing is auto-approved. Timeouts, cancels, closes and disconnects deny.
- `Composer::submit` is the only way user-typed text is sent. `check_usage` is the only other sender and sends only the constant `/usage`. Notification drafts need a Send click.
- Agent text is untrusted. It never becomes markup, a command, a path or a setting.
- Nothing in the repo reads Claude's login file or token or calls private endpoints.
- The panel never runs a command that applies system changes. Applying changes stays a terminal action: `just rebuild`.
- The managed policy (nix/policy.nix) only changes through an item that says so.

---

## 2. Already done (for context, do not redo)

Spec and spike. ACP core with mode pinning. View-model and GTK panel. GNOME extension (Super+F12, Quick Settings tile, docked drop-down with float toggle). Module install gated on the managed policy. Notifications with explicit Send. Draft flow hardening. Markdown rendering with tables. Launch-time 5-hour usage line that is also the Check usage button. Credential probe, accounts and autoswitch removed. Rebuild button removed. Panel starts the saved agent with no picker. Shell-native design note and Phase 0 spike (passed in the nested shell, with conditions). Windows CI job and platform boundary note. CLAUDE.md migrated.

---

## 3. Phase A: Decisions and user tasks (nothing downstream starts without these)

- [x] **A1 USER** — done 2026-10-31: user ran every step of the spike on the VM and all worked (opens fast, typing normal, focus, Alt+Tab, error test, lock, odd text), EXCEPT the slide: there was no slide animation on the VM, cause unknown. `enable-animations` is `true` on the VM, so it is not the animation setting. The GTK window showed no slide either (C2). Carried into B2 as a diagnosis item. (Original: Try the Phase 0 spike on the VM: follow `spike/shell-panel/README.md`. Remove it afterwards.)
- [x] **A2 DECISION** — done 2026-10-31: user chose CONTINUE TO PHASE B (Shell-native panel).
- [x] **A3 DECISION** — done 2026-10-31: user ACCEPTS weaker approvals in the Shell-native design, with three conditions: (1) the helper accepts `Decide` only from GNOME Shell's own connection, checked by process id, with a test proving another process is refused; (2) `Decide` works only for a currently pending request, together with the generation number; (3) plan managed ask rules for `gdbus`, `dbus-send` and `busctl` in `nix/policy.nix`, but do NOT change the policy yet: ask the user when that item comes up. Reason: Omarchy has no approval UI to copy; its agent widget reportedly runs with permissions bypassed, so we do not follow it there. (Original: Accept weaker approvals in the Shell-native design? See `shell_native_panel_spec.md`.)
- [x] **A4 DECISION** Keep the GTK window as a fallback when the extension is missing (recommended), or require the extension. — done 2026-10-31: user chose to REQUIRE the extension (no GTK fallback). Makes B10 unnecessary and B13 apply.
- [x] **A5 DECISION** (done 2026-10-31: user confirmed END AT CLOSE) Helper lifetime: end the session and exit at close (recommended), or a short idle timeout. The user rejects anything resident or started at login. — 2026-10-31: user asked how Omarchy does it. Research: Omarchy has no resident assistant; each agent starts on demand in a terminal and ends when it closes (docs: omacom/omarchy manual 17-ai.md). That matches "end at close". Awaiting the user's final answer.
- [x] **A6 DECISION** Keep OpenCode or drop it. It has never been tested on the real host and has one open route (a custom agent the user selects). — done 2026-10-31: user chose to DROP OpenCode. E3 is unblocked (remove it, map first).
- [x] **A7 DECISION** Add vexos-ai to the Daily workflow in vexos-nix, with an eval gate or a PR? (Other repo, see Handoffs.) — done 2026-10-31: user chose yes, with an eval gate. X2 is unblocked.
- [ ] **A8 DECISION** Windows path: native app, or an installer that sets up WSL. Also UI toolkit and .msi build and signing. Later.
- [x] **A9 DECISION** — done 2026-10-31: bus messages with the caller check; compare a private socket in B1 before building (done in B1 below). Drop the failed-service and coredump watchers from the panel for now (keep the draft code). NOT answered: the D-Bus crate choice (Decision 7 in the note, a new dependency, Tier 3): decide it at the start of B4 with a comparison of at least two options. (Original: bus methods versus a private socket, the D-Bus crate, and what to do with notification watchers.)

---

## 4. Phase B: Shell-native panel (only if A2 = continue; Tier 3)

The panel is drawn inside GNOME Shell with St widgets. A headless Rust helper holds the session and the safety rules. Read `shell_native_panel_spec.md` fully before B1.

- [~] **B1** — done 2026-10-31 as a PROPOSAL: the note is read (sections 3 to 9 and "Phase 0 results") and B2 onward is expanded below. Waiting for the user's approval; nothing from B2 on is started. Order below is the order of work. Every item is Tier 3 unless it says otherwise (Tier 3 = plan, wait for approval, independent reviewer, mutation-check any safety test).

  **Where this list differs from the note (user to approve with the list):**
  1. The note's `ui` setting (`window` default, `shell`) comes back as a TEMPORARY dev switch: the GTK panel stays installed and untouched until B13, then the switch and the GTK panel go (A4: the extension is required, no fallback).
  2. The helper uses a NEW bus name, `io.github.victorytek.VexosAssistant.Helper`, not the GTK panel's name. Reason: the GTK panel owns the old name and its activation file until B13, and the only reason to keep the old name (notification clicks) is gone with the watchers (A9).
  3. The process-id check (A3 condition 1) is required, not optional. Honest limit: the helper checks that the caller's connection is the one owning `org.gnome.Shell`, that its uid is the user's, and that the process behind it is `gnome-shell` (from `GetConnectionCredentials`, then the `/proc/<pid>/exe` link). Other extensions run inside the same process, so this proves "it is the Shell", not "it is our extension". The note said this too.
  4. Numbers from the Phase 0 results replace the note's: a reply or approval card is shown at most 16 KiB (after escaping), not 64 KiB; a `Send` is at most 60,000 bytes, not 72 KiB (the input holds 65,535 bytes).
  5. Approval cards also escape the hidden characters the E4 fix added (tag characters, variation selectors, U+2060-2064), not only control and bidi characters.
  6. The helper registers NO `diagnose-*` actions: with the watchers gone nothing triggers a draft in Shell mode. The draft code in the view-model stays. Say if you want them registered.
  7. Still open: the D-Bus crate (B4a); how the helper learns `usage_on_launch` (B9).

  **Private socket versus bus methods (A9; done here, nothing built):** The bus design needs `Send` and `Decide` on the bus, protected by the sender check on every call. The socket design (`Open()` returns one end of a socketpair; everything else runs on it) takes `Send` and `Decide` off the bus and hides the traffic from bus monitors. Against a hostile OTHER PROCESS the sender check already refuses every call, so the socket adds only confidentiality (a same-user process may be able to read the conversation off the bus; unconfirmed whether the real session bus allows monitoring). Against a hostile EXTENSION (same process as the Shell) neither helps, because it can call `Open()` and get the socket. Against a fake helper neither helps (B6c deals with it). The socket costs a framing protocol, fd passing in GJS (unconfirmed it works from an extension), a parser inside the privileged Shell, and harder tests. Verdict: build the bus methods first, behind a thin transport layer (a Rust trait in the helper, one JS module in the extension), so a socket can replace it later; revisit if bus monitoring turns out to work on the VM (B12 checks it).

- [ ] **B2a** (Tier 2, diagnosis only; Needs: the user runs a few commands on the VM) Find out why the spike's slide did not show on the VM, with evidence. Animations are enabled there. Ask the user for `gnome-shell --version`, `echo $XDG_SESSION_TYPE`, the VM type and whether it has a GPU, and `journalctl --user -b -g vexos-shell-spike`; read how the spike records its frames (`_trace`); report options. Same symptom as C2 (GTK window).
- [ ] **B2b** (Needs: B2a not showing a blocker) Fix the spike's selected-text colour (white on light) and show it in the nested shell with a screenshot.
- [ ] **B2c** (Needs: user's yes) Undo in the input: recommend "not supported", written into the note and shown nowhere in the UI. Say so; do not build undo.
- [ ] **B2d** Input byte cap: detect a paste cut at Clutter's 65,535 bytes, show a notice, and refuse `Send` until it is shorter than 60,000 bytes. Test in the spike.
- [ ] **B2e** Enter versus input methods: read how Shell 50's own entries handle Enter during composition, make Enter never send while a composition is open, and test it as far as the nested shell allows. If IME cannot be tested here, mark it untested and put it in the VM checklist (B12).
- [ ] **B3a** (view-model, also changes the GTK panel; Needs: B1 approved) Block structure for rendering: an additive block-level output from `markdown()` (paragraph, heading, list item, code block, table). The existing run output stays for the GTK panel. Tests first; mutation-check.
- [ ] **B3b** (view-model) A card whose content is over 16 KiB after escaping is created already denied ("too large to review") and offers no buttons. Tests first; mutation-check; the GTK panel gets this too.
- [ ] **B3c** (view-model) Escape control, bidi and the E4 hidden characters in card content as visible `\u{…}`, and a literal backslash as `\\`; one shared function with `sanitize`. Tests first; mutation-check.
- [ ] **B4a DECISION** (Tier 3, new dependency; Needs: B1 approved) The D-Bus crate. Compare at least two (for example zbus, and the `gio` D-Bus API the panel already links), read each one's current docs, release notes and advisories, and check fd passing and private-bus testing. Show the user a comparison and wait for the choice.
- [ ] **B4b** (Needs: B3, B4a) New headless crate `vexos-ai-helper`: owns `Session`, `Conversation` and `Composer`, no window, new bus name, `serde` on the display types it sends (additive). Exits when no `Open()` follows activation. Needs core API only as-is; if a core change is needed, stop and ask.
- [ ] **B4c** (Needs: B4b) The caller check: every method refused unless the caller is the owner of `org.gnome.Shell`, same uid, process is `gnome-shell`; owner tracked through `NameOwnerChanged`; if the Shell's name loses its owner, deny everything pending and end the session. Test with a second process on a private test bus calling EVERY method (all refused), and with a stand-in "Shell" (the executable name is a test setting).
- [ ] **B4d** (Needs: B4c) Methods `Open`, `Close`, `Edited`, `Send`, `Cancel`, `CheckUsage`, `DraftChoice`, `DraftDiscard`. `Send` reaches only `Composer::submit`; `CheckUsage` takes no argument. Size and rate limits (60,000 bytes for `Send`/`Edited`; rates from the note, section 3.4). A test pins the method list so no method can be added silently.
- [ ] **B4e** (Needs: B4c) `Decide(generation, request, choice)` and `Shown`: A3 conditions 1 and 2. Only `"approve-once"` and `"deny-once"`; only a request pending right now; the generation must match; refused for a card not reported shown; `approve-once` refused until 400 ms after `Shown`; refused while the panel is closed. Tests: wrong caller, non-pending id, stale generation, `"approve-always"`, unshown, too early. Mutation-check each rule.
- [ ] **B4f** (Needs: B4d) Display-only signals sent to the Shell only: `State`, `Item`, `Usage`, `Draft`, `Ended`. Payload caps from the note; streaming coalesced.
- [ ] **B4g** (Needs: B4d, B4e) Source scans for the helper (one `prompt` call, `Outgoing` made only in `submit` and `check_usage`, fixed argv only, no way to set a mode, agent, path or command, no registered actions) and the mutation checks for them. Then the independent reviewer on the whole B3-B4 diff.
- [ ] **B5a** (Needs: B4g) Packaging: D-Bus service file, module option (off by default), `checks.module` cases (the module still never enables the extension; the service `Name=` rule), `checks.extension` updated. `nix/policy.nix` is NOT touched here.
- [ ] **B5b DECISION** (Tier 3, `nix/policy.nix`; A3 condition 3) STOP and ask the user before editing. Propose managed ask rules for `gdbus`, `dbus-send` and `busctl` (and their wrappers, as E2 tested), the matching `checks.policy` asserts, and a run of the E2 harness against them. Wait for the user's yes before any change to the policy.
- [ ] **B6a** (Needs: B2, B5a) Pure JS modules with no `gi://` imports: payload validators and caps, block to style-descriptor mapping, button-list checks, geometry. Node tests; mutation-check.
- [ ] **B6b** (Needs: B6a) The panel in the extension behind the temporary `ui` switch: St widgets, runs built from validated pieces, no markup from text, display caps (16 KiB per message, 2,000 lines, 200 rows), the slide started after the first painted frame, modal grab as the spike proved, Copy button, guarded handlers.
- [ ] **B6c** (Needs: B6b; research first) D-Bus client in the extension: async calls only, `Edited` flushed before a `Draft` is applied, no echo of helper text. Also the fake-helper problem: before `Open()` the extension checks who owns the helper's name (credentials, process path) or the helper is started as a systemd user unit; first find out what the JS side can check, then pick the smaller one.
- [ ] **B6d** (Needs: B6c, B4e) Approval cards drawn only from `ApprovalCard`: `Shown` after drawing, Approve disabled until 400 ms after the `Shown` reply, a visible notice on any refused `Decide`, over-cap cards never drawn.
- [ ] **B6e** (Needs: B6d) `disable()` closes the session, pops the grab and destroys every actor (screen lock). Extension source scans updated (no markup, eval, spawn, file or network use). Nested-shell tests against a FAKE helper (canned data, records its calls) including hostile payloads, and the frame test for the slide. Then the independent reviewer on the extension diff.
- [ ] **B7** (Needs: B6e) Lifecycle: `Open` starts the helper and the session; `Close` ends the session and the helper exits (A5); pending approvals are denied on close, on the Shell vanishing and on a helper crash ("The assistant stopped"). Nothing runs while idle except the extension. Bus tests and a nested-shell test.
- [ ] **B8** (Needs: the user's yes to the map) Watchers (A9: dropped for now): map first what is watcher code and what is draft code (`diagnose.rs` and the composer draft flow stay; `notify.rs` in the view-model and the panel, the module and settings options, the scans and docs are checked line by line), show the user the list of what goes and what stays, then remove the watchers from the GTK panel. Also closes D3.
- [ ] **B9** (Needs: B4f, B6c) Usage line and Check usage in the Shell panel: same strict parser in the helper, `usage_on_launch` (E4 #5: on by default, opt-out) read by the helper (find the smallest way and ask if it needs a new setting), and `CheckUsage()` still only the constant `/usage`.
- [x] **B10** — superseded 2026-10-31 by A4 (the extension is required, no GTK fallback); nothing to do.
- [ ] **B11** (Needs: B7; ask before editing `docs/contract.md`) Contract: list whether anything in `docs/contract.md` or the Bats suite changes. The helper is not the CLI, so the likely answer is "only wording about the panel". Minimum change, paired; no safety statement weakened.
- [ ] **B12** (Needs: B9) Write the real-screen checklist for the user (VM): opens fast, the slide (B2a), typing, IME, screen lock, other extensions, multi-monitor, HiDPI, whether bus monitoring can read the conversation. Then the user tests on the VM.
- [ ] **B13** (Needs: the user confirms the Shell panel works on the VM; Tier 3, large deletion, ask first) Remove the GTK panel: `crates/vexos-ai-panel`, `nix/panel.nix`, `window.rs`, the extension's window tracking, the `window-mode` setting and the temporary `ui` switch, the dock-icon workarounds; update `flake.nix` checks, docs and the Bats/scan tests that named them.

---

## 5. Phase C: Fix the GTK window (only if A2 = stop or change course; shelved otherwise)

- [ ] **C1** Cold start speed. The whole window appears at once a few seconds after Super+F12 with no Starting state first. Terminal launch is faster but not instant. The user rejects a resident or hidden process and any autostart; the X button keeps quitting. Map the path (D-Bus activation, wrapper script, GTK start-up, stylesheet, window build), add timing points, compare the default renderer with `GSK_RENDERER=cairo` (VMs often lack a GPU), and report options with costs.
- [ ] **C2** Slide-down is not visible on the real VM; the window just appears. The nested test was misleading. Find the cause with evidence. Ask the user for: whether a second show is instant, `gnome-extensions info vexos-assistant@victorytek.github.io`, and `journalctl --user -b | grep -i vexos`.
- [ ] **C3** Dock icon: the user wants no icon, no Alt+Tab entry and no overview entry, like Quick Settings. Research what a Shell extension can do on GNOME 50 Wayland; implement the smallest safe version or describe options.

---

## 6. Phase D: Features

- [ ] **D1** Search: write a short design note (docs only). What is worth searching (conversation history, system state, rebuild logs), what data it needs, privacy, and where it runs under the chosen front end.
- [ ] **D2** (needs D1 and user approval) Implement search.
- [ ] **D3** (A2 and A9 decided 2026-10-31: the watchers are dropped from the panel for now, so this closes with B8; revisit if the user wants them back) Failed-service and coredump notifications: they are built but off by default. Decide how they run with no resident panel, then adapt or remove.

---

## 7. Phase E: Safety and testing

- [~] **E1 USER** (reading guide written 2026-10-31: `.github/docs/subagent_docs/composer_reading_guide.md`; waiting for the user to read composer.rs) Read `crates/vexos-ai-viewmodel/src/composer.rs` once. It decides what the panel can send. Claude Code: first write a short reading guide (which functions matter, what to look for) in `.github/docs/subagent_docs/composer_reading_guide.md`.
- [~] **E2** — run 2026-10-31 (claude-agent-acp 0.88.0, session started in acceptEdits, managed policy vs no-policy control, 11 cases each, real agent in the bwrap sandbox, scratch in `target/spike-e2/`). With the policy, every write path asked and nothing slipped through: `timeout touch`, `env touch`, `nice touch`, `bash -c 'touch …'`, `echo … | xargs touch`, `echo > file`, `echo | tee file`, `Edit`, `Write` and `NotebookEdit` each raised an approval request (the client rejected it; no file changed). Without the policy (control), `timeout touch`, `nice touch`, `echo > file`, `Edit`, `Write` and `NotebookEdit` ran with no request in acceptEdits, so the ask rules are what stops them (`env touch`, `bash -c`, `xargs`, `tee` asked even without it). Nothing was denied outright (denies are only for the apply commands, not tested here). Missing: `MultiEdit` could not be tested, because this adapter version does not offer the tool (the agent said so); the ask rule for it is harmless but unproven. Only default-mode-in-acceptEdits was run, not plan mode. Original: Test the ask rules against MultiEdit, NotebookEdit and wrapper commands (`timeout touch`, `env touch`, `nice touch`) in the spike harness. Report which ask or are denied and which slip through.
- [x] **E3** — done 2026-10-31: OpenCode removed from CLI, module, policy, core, view-model, contract and tests; `pick` kept (claude only), a leftover `opencode` agent file launches Claude and shows null in status; `programs.vexos-ai.opencodePackage` is now a removed option with a clear error. Handoffs X2/X4/X5 below. (original text:) If OpenCode is dropped: remove it from the panel, the module, the contract and the tests (map first). If kept: document the custom-agent route as accepted in docs/contract.md.
- [x] **E4** — closed 2026-10-31 with the user's decisions: #1 and #2 fixed; #3 skipped (revisit for Windows, H2: killing the agent's process group is a core change, and the Windows launcher differs); #4 skipped (revisit if the GTK panel stays as the main path); #5 kept as is (the launch `/usage` stays on by default, opt-out via `usage_on_launch`). Details: reviewed 2026-10-31 by a fresh reviewer subagent: PASS, no CRITICAL (no path found that sends text or approves without a user action; the one send with no click is the constant `/usage` at launch, by design). FIXED 2026-10-31: #1 (Send now has `focus_on_click(false)`, pinned by a scan test; Tab still reaches Send) and #2 (`is_invisible` now covers U+2060-2064, U+180B-180F, variation selectors and tag characters; test added; both mutation-checked; independent review PASS). #3 NOT changed: `Session::drop` already kills the agent at once (`start_kill`, `kill_on_drop`), so a `cancel()` before it would race, and only processes the agent itself started could outlive it; a real fix (kill the process group) is a core change and needs the user's decision. #4 and #5 still open. Five RECOMMENDED findings were: (1) Send button has no `focus_on_click(false)`, so a focused Send + a silently placed draft + Enter/Space sends an unreviewed draft (`panel/src/main.rs` ~315, 482-497); (2) `sanitize` in `viewmodel/src/diagnose.rs` ~51 misses invisible Unicode (tag characters U+E0000-E007F, U+2060-2064, variation selectors), so hidden text in logs can reach the review draft; (3) `close_session` (`main.rs` ~887) drops the session without `cancel()`, so a running turn keeps going after the window closes (no approve path; a late request is denied after 600 s); (4) `scan_tests.rs` can be evaded (`Session::prompt(&s, ..)` call syntax, macros, new unscanned files, `Self(s)` or `Outgoing { 0: s }`, no scan for `decide(`); (5) the launch `/usage` is opt-out, and any session-bus caller of `app.toggle` or `app.diagnose-*` can start it. Not examined: extension, rpc.rs, agent.rs, nix/policy.nix, runtime GTK focus, AT-SPI. Original: Independent review of the send gate: composer.rs, check_usage, the source scans, and the helper's Send and Decide paths if Phase B ran. Use a fresh reviewer subagent with no prior context and ask it to try to find a path that sends text without a user click.
- [x] **E5** — done 2026-10-31: no change, reasons recorded. User decided to leave policy denies showing "failed". (report only, nothing changed) Finding from the recorded live runs in `spike/acp/out/` (claude-agent-acp 0.88.0): a managed-policy deny arrives as a failed tool update with no permission request, with `_meta.claudeCode.nonExecutionKind = "permission-rule"` and text "Permission to use Bash with command … has been denied." But a deny the USER gave also carries `nonExecutionKind = "permission-rule"` (text "User refused permission to run tool"), and so does the no-policy control, so that field alone does not mean "policy". It could still be used together with the fact that no approval card exists for the call (the panel already knows that), which would mean "a permission rule denied it" (managed policy, or the user's own deny rules; the two cannot be told apart). The core drops `_meta` today (`session.rs` reads only id, title, kind, status), so showing it needs an additive core change (a new field on `ToolCall`) and is a protocol detail of one adapter version. Nothing was changed. Decision for the user: leave as "failed" (current), or add the additive core field and show "denied by a rule". OpenCode is gone, so only Claude matters. Original: Policy denies show "failed" because the core cannot tell them apart. Check whether the adapter exposes anything that lets the panel say "denied by policy". Do not guess. Report only.
- [x] **E6 DECISION** — done 2026-10-31: user chose to SKIP. No `Event::Usage` and no context-window line. Original: Event::Usage (context-window line). Core change, additive, numbers only. Recommended: skip.

---

## 8. Phase F: Small cleanups in this repo

- [ ] **F1** Markdown tables wider than the window lose alignment. Make them scroll or wrap sensibly in the front end that ships.
- [x] **F2** — done 2026-10-31: every code comment and the other specs already mark removed features; only `assistant_panel_spec.md` had no banner, so it got one (accounts, picker, OpenCode, rebuild button, usage cache). Original: Search docs and comments for statements about removed features (accounts, autoswitch, usage probe and timer, rebuild button, agent picker, "never start a session from a notification") and fix or mark them superseded.
- [x] **F3** — done 2026-10-31: `/target` and `/result*` are gitignored; `spike/acp/out/` is ignored too (by its own rule). No `target/tools/dl` exists and no `/tmp/bats-run-*` or `/tmp/vexos-ai-spike-*` are left. Delete by hand if you want the space: `target/` (about 8.4 GB). Original: Make sure target/ scratch and download folders are gitignored. List anything the user should delete by hand.

---

## 9. Phase G: Rollout (user tasks)

- [ ] **G1 USER** After every vexos-ai push: in vexos-nix run `nix flake update vexos-ai`, commit and push. Check the new rev matches the latest vexos-ai commit.
- [ ] **G2 USER** Test on the VM first: "Up", `just rebuild`, reboot, log out and in.
- [ ] **G3 USER** Laptop: enable the AI feature in `/etc/nixos/features.nix`, "Up", rebuild, reboot. If the extension is not on, run `gnome-extensions enable vexos-assistant@victorytek.github.io`.
- [ ] **G4 USER** On the laptop, from a normal terminal (not VS Code): run `scripts/verify-host.sh` from a clone at the pinned rev. Then in the panel: `nixos-rebuild switch --dry-run` must be refused with no card; `touch /tmp/panel-test` must show a card with only Approve once and Deny once.

---

## 10. Phase H: Windows (later; keep one project)

- [ ] **H1 USER** Open the first run of `windows-portable.yml` in Actions and paste the failing step. The job is non-blocking on purpose.
- [ ] **H2** Fix what H1 shows in the portable crates. Test-fixture problems (like the Unix `/tmp` working directory in core tests) are test fixes. Anything that changes how the core launches agents is a core change: stop and ask. Revisit E4 #3 here (kill the agent's whole process group on close).
- [ ] **H3 DECISION** (A8) Native app or WSL installer; UI toolkit; .msi build and signing.
- [ ] **H4** (needs H3) Implement the platform boundary from `platform_boundary_spec.md`: traits for apply action, diagnostics, notifier, policy locations, agent launcher. Linux implementation first, no behaviour change.
- [ ] **H5** (needs H3) Windows policy install and repeat the spike escalation tests on Windows. The Linux sandbox harness does not work there.

---

## 11. Handoffs to other repos (Claude Code: you cannot edit these)

For each item, write the full prompt here (with the scope rule, verification and one commit message), tell the user which repo it belongs to, and wait for them to report back.

**Paste the "Common rules" block first, then one prompt, into a Claude Code session opened in the other repo.** Written 2026-10-31. After the session finishes, tell me what it reported.

<details><summary>Common rules (prepend to every prompt)</summary>

```
Work only inside this repository's directory. Do not read or write other repos, ~/.claude, ~/.config, dconf or /etc, and never read credential files or tokens. Do not run nixos-rebuild, just rebuild/switch/update, nix flake update, nix flake check, nix-collect-garbage, or anything with sudo. Do not run git add, commit, push or stash: I commit with GitHub Desktop. Read this repo's CLAUDE.md first and follow it where it does not conflict with these rules; if it asks for something these rules forbid, say what you skipped. Do one task at a time, read the files before changing them, and change only what the task needs. Verify with this repo's own build and test commands (run only what applies, report exit codes, and write "-" for anything you did not run). Do not call anything done unless it ran. If something contradicts the task, stop and tell me. Finish with ONE commit message in this repo's commit format that covers every uncommitted change, and keep the report short and in plain language.
```

</details>

- [ ] **X1 VexPortal repo** Small cleanup.

  ```
  Task: small cleanup in VexPortal. Check each item first and fix only what is true.
  1. CLAUDE.md says no preflight script exists, but one does. Fix the statement (and any command list that should name it).
  2. Five older drift items need a look: `_service-info`, `beszel`, `clean`, `rust-repair`, `static-ip`. For each, find what drifted (a name, a recipe or a doc that no longer matches the code or the vexos-nix justfile). Report what you found; fix it only if the fix is mechanical and obvious, otherwise list it for me.
  3. The dialog text "Your sign-ins are kept either way" may be dead since the Claude accounts feature was removed from vexos-ai. Find where it is shown. If nothing can show it any more, remove it; if something still can, leave it and say why.
  4. The test `reads_this_user_without_erroring` has no assertion. Add a meaningful one (what does "without erroring" mean for that function?), or if it cannot assert anything real, say so and tell me what to do.
  Verify with this repo's preflight script and test command.
  ```

- [ ] **X2 vexos-nix repo** (A7 decided: yes, with an eval gate) Add vexos-ai to the Daily workflow.

  ```
  Task: add the vexos-ai flake input to this repo's "Daily workflow" (the justfile recipes the user runs every day, such as the "Up" / update recipe), with an evaluation gate.
  Context: vexos-ai is an install-only flake consumed as an input; this repo pins the same nixos-26.05 revision. Updating the input should be a deliberate step: `nix flake update vexos-ai`, then commit and push. The user wants a gate that evaluates a few host variants before anything is pushed, so a vexos-ai change cannot break a host unnoticed.
  First read: the justfile, how the Daily/Up flow works now, how other inputs are updated, and how `dry-build` is used. Then propose the smallest change in chat before editing: which recipe changes, which variants are evaluated (pick a few that cover different roles, e.g. one desktop role with the AI feature on, one without), and what happens when the gate fails. Wait for my yes before you edit.
  Rules: never run `just rebuild|switch|update|update-all`; validate with `dry-build` only. Do not change flake.lock yourself.
  ```

- [ ] **X4 vexos-nix repo** (E3 done) Follow vexos-ai dropping OpenCode.

  ```
  Task: vexos-ai removed OpenCode (2026-10-31). Claude Code is the only assistant. `programs.vexos-ai.opencodePackage` is now a removed option: setting it fails evaluation with "OpenCode was removed from vexos-ai; delete this setting."
  In this repo:
  1. In modules/ai.nix (and anywhere else it appears), delete the `opencodePackage` setting and any `pkgs.unstable.opencode` use made only for it.
  2. Remove OpenCode wording, tests, docs or package pins that no longer apply. Search the whole repo for "opencode" case-insensitively and list each hit with what you did.
  3. Do not touch the feature toggle, role gating or the pkgs.unstable choice for Claude packages.
  The flake.lock bump to the new vexos-ai revision is a separate, user-run step (`nix flake update vexos-ai`). Do not run it. This change must land together with that bump, because until the lock is bumped the old vexos-ai still expects the option.
  Verify with `dry-build` for a few host variants (AI feature on and off). Never rebuild or switch.
  ```

- [ ] **X5 VexPortal repo** (E3 done) Follow vexos-ai dropping OpenCode.

  ```
  Task: vexos-ai removed OpenCode (2026-10-31). `vexos-ai pick opencode` now exits 2 with "unknown assistant 'opencode' (use claude)". `vexos-ai pick claude` still works, and `status --json` is unchanged (schema 1). An old `opencode` value in ~/.config/vexos/ai/agent now shows as `agent: null` in status and launches Claude.
  In this repo:
  1. Remove opencode from the assistant catalog and from every place VexPortal offers or calls `vexos-ai pick opencode`.
  2. Make the UI and code cope with `agent == null` (nothing chosen) and with Claude as the only choice, without a leftover one-item picker that looks broken (decide the simplest honest UI: for example just show Claude, or keep a "set up" action that runs `vexos-ai pick claude`).
  3. Update tests that mention opencode, and any docs.
  The contract this relies on is docs/contract.md in the vexos-ai repo (argv, exit codes, `status --json`). Do not read that repo; the facts above are all you need.
  Verify with this repo's preflight script and tests.
  ```

- [ ] **X3 vexos-nix repo** (needs Phase B) If the Shell-native panel ships: the enabled-extensions handling and any new module option wiring.

---

## 12. Things the user deletes by hand

`~/.claude/plans/` old plan files. `~/.local/share/vexos/ai/claude-accounts/`. `/tmp/bats-run-*` and `/tmp/vexos-ai-spike-*`. `target/` scratch folders. `target/tools/dl/`. `~/.config/opencode/` (the old theme file `tui.json` and anything else OpenCode left).

---

## 13. Log (newest first; one line per item touched)

- 2026-10-31 A1, A2, A3, A9 decided and marked done. B1 expanded (proposal, awaiting approval); B10 superseded by A4.
- 2026-10-31 A1 reported by the user: opens fast, typing normal, no slide (same as the GTK window, C2). A2 is now ready for the user's decision.
- 2026-10-31 E4 closed (#3 skipped, revisit for H2; #4 skipped, revisit if the GTK panel stays main; #5 kept). E6 closed: skipped.
- 2026-10-31 E4: findings #1 and #2 fixed (review PASS, preflight 0); #3 explained, no change; #4 and #5 open.
- 2026-10-31 E2 run: all write paths ask under the policy; MultiEdit not testable (tool absent in adapter 0.88.0).
- 2026-10-31 E4 reviewed: PASS with 5 RECOMMENDED findings awaiting the user's decision.
- 2026-10-31 E5 closed: user chose to leave "failed"; no change.
- 2026-10-31 E5 reported: the adapter does expose `nonExecutionKind: permission-rule`, but it does not separate policy from user denies.
- 2026-10-31 E1: reading guide written (user still to read). X1, X2, X4, X5: handoff prompts written.
- 2026-10-31 F2 done: banner added to assistant_panel_spec.md.
- 2026-10-31 F3 done: scratch folders are ignored; nothing to remove but `target/`.
- 2026-10-31 E3 done: OpenCode removed (see item).
- 2026-10-31 A4 decided: require extension, no GTK fallback.
- 2026-10-31 A5 confirmed: end at close. A1 deferred by the user.
- 2026-10-31 A5 researched (Omarchy is on-demand, no resident process); awaiting final answer.
- 2026-10-31 A6 decided: drop OpenCode.
- 2026-10-31 A7 decided: add to Daily workflow with an eval gate.
- 2026-10-31 plan created.