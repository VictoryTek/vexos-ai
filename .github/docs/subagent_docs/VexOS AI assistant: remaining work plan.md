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

- [ ] **A1 USER** (deferred 2026-10-31: user will test later, further along; A2 stays open until then) Try the Phase 0 spike on the VM: follow `spike/shell-panel/README.md` (copy, log out and in, enable, Super+F10). Report: opens fast? slide smooth? typing normal? anything wrong? Remove it afterwards.
- [ ] **A2 DECISION** (needs A1) Shell-native panel: continue to Phase B, change course, or stop and fix the GTK window (Phase C). The note recommends continuing with conditions.
- [ ] **A3 DECISION** Accept weaker approvals in the Shell-native design? Approval becomes a message on the session bus, so the trust boundary includes the Shell and its extensions. See `shell_native_panel_spec.md`. The policy and deny rules still hold.
- [x] **A4 DECISION** Keep the GTK window as a fallback when the extension is missing (recommended), or require the extension. — done 2026-10-31: user chose to REQUIRE the extension (no GTK fallback). Makes B10 unnecessary and B13 apply.
- [x] **A5 DECISION** (done 2026-10-31: user confirmed END AT CLOSE) Helper lifetime: end the session and exit at close (recommended), or a short idle timeout. The user rejects anything resident or started at login. — 2026-10-31: user asked how Omarchy does it. Research: Omarchy has no resident assistant; each agent starts on demand in a terminal and ends when it closes (docs: omacom/omarchy manual 17-ai.md). That matches "end at close". Awaiting the user's final answer.
- [x] **A6 DECISION** Keep OpenCode or drop it. It has never been tested on the real host and has one open route (a custom agent the user selects). — done 2026-10-31: user chose to DROP OpenCode. E3 is unblocked (remove it, map first).
- [x] **A7 DECISION** Add vexos-ai to the Daily workflow in vexos-nix, with an eval gate or a PR? (Other repo, see Handoffs.) — done 2026-10-31: user chose yes, with an eval gate. X2 is unblocked.
- [ ] **A8 DECISION** Windows path: native app, or an installer that sets up WSL. Also UI toolkit and .msi build and signing. Later.
- [ ] **A9 DECISION** Remaining open decisions in section 11 of the Shell-native note: bus methods versus a private socket, the D-Bus crate, and what to do with notification watchers.

---

## 4. Phase B: Shell-native panel (only if A2 = continue; Tier 3)

The panel is drawn inside GNOME Shell with St widgets. A headless Rust helper holds the session and the safety rules. Read `shell_native_panel_spec.md` fully before B1.

- [ ] **B1** Read the note's migration section and the Phase 0 results. Expand B2 onward into concrete sub-items here, each with Needs and Tier. Show the user the expanded list and wait for approval before B2.
- [ ] **B2** Fix the problems the spike found: selected reply text unreadable (white on light); no undo in the input (implement or document as not supported); the Enter-versus-input-method risk (Enter must never send in the middle of a composition; test if possible, otherwise mark untested); handle Clutter's 65,535 byte input cap.
- [ ] **B3** View-model changes the note requires: block structure for rendering; deny at once for oversized approval cards; escaping of control and bidi characters. These also change the GTK panel. Mutation-check.
- [ ] **B4** Headless helper (new crate, no window): the session D-Bus service per section 3 of the note. Methods Open, Close, Send(text) that only reaches Composer, Decide(id, choice) accepting only "approve-once" and "deny-once" for a request that is currently pending, CheckUsage with no argument, Cancel, Discard. Size and item caps. Caller check. Generation number. Minimum delay before Approve. Display-only signals. List what the extension must never be able to do. Needs core API only as-is; if a core change is needed, stop and ask.
- [ ] **B5** Packaging: D-Bus service file, module option, checks.module and checks.extension updated. The module still never enables the extension.
- [ ] **B6** The real Shell panel in the extension: St widgets, runs built from validated pieces, no markup from text, display caps (16 KiB per message, 2,000 lines, 200 rows unless the note changed them), guarded handlers, modal keyboard grab, full cleanup in disable(). The extension holds no safety logic.
- [ ] **B7** Lifecycle: Open starts the helper and the session; Close ends the session and the helper exits; pending approvals are denied on close. Nothing runs while idle except the extension.
- [ ] **B8** Notifications under the new design (needs A9): move watchers out of the panel process (systemd user units) or drop them. Clicks open the panel with an unsent draft.
- [ ] **B9** Usage line and Check usage inside the Shell panel, same strict parser, same rules.
- [ ] **B10** (needs A4) Keep the GTK window working without the extension, and make sure both front ends share the same view-model and helper rules.
- [ ] **B11** Update docs/contract.md and the Bats suite together, minimum change, paired. List each change. Do not weaken any safety statement.
- [ ] **B12** Write the real-screen checklist for the user (VM): opens fast, slide smooth, typing, IME, screen lock, other extensions, multi-monitor, HiDPI.
- [ ] **B13** If the GTK window is no longer the main path: remove the slide and placement code that only served it, and the dock-icon workarounds. Only after the user confirms the Shell panel works on the VM.

---

## 5. Phase C: Fix the GTK window (only if A2 = stop or change course; shelved otherwise)

- [ ] **C1** Cold start speed. The whole window appears at once a few seconds after Super+F12 with no Starting state first. Terminal launch is faster but not instant. The user rejects a resident or hidden process and any autostart; the X button keeps quitting. Map the path (D-Bus activation, wrapper script, GTK start-up, stylesheet, window build), add timing points, compare the default renderer with `GSK_RENDERER=cairo` (VMs often lack a GPU), and report options with costs.
- [ ] **C2** Slide-down is not visible on the real VM; the window just appears. The nested test was misleading. Find the cause with evidence. Ask the user for: whether a second show is instant, `gnome-extensions info vexos-assistant@victorytek.github.io`, and `journalctl --user -b | grep -i vexos`.
- [ ] **C3** Dock icon: the user wants no icon, no Alt+Tab entry and no overview entry, like Quick Settings. Research what a Shell extension can do on GNOME 50 Wayland; implement the smallest safe version or describe options.

---

## 6. Phase D: Features

- [ ] **D1** Search: write a short design note (docs only). What is worth searching (conversation history, system state, rebuild logs), what data it needs, privacy, and where it runs under the chosen front end.
- [ ] **D2** (needs D1 and user approval) Implement search.
- [ ] **D3** (needs A2 and A9) Failed-service and coredump notifications: they are built but off by default. Decide how they run with no resident panel, then adapt or remove.

---

## 7. Phase E: Safety and testing

- [ ] **E1 USER** Read `crates/vexos-ai-viewmodel/src/composer.rs` once. It decides what the panel can send. Claude Code: first write a short reading guide (which functions matter, what to look for) in `.github/docs/subagent_docs/composer_reading_guide.md`.
- [ ] **E2** Test the ask rules against MultiEdit, NotebookEdit and wrapper commands (`timeout touch`, `env touch`, `nice touch`) in the spike harness. Report which ask or are denied and which slip through.
- [x] **E3** — done 2026-10-31: OpenCode removed from CLI, module, policy, core, view-model, contract and tests; `pick` kept (claude only), a leftover `opencode` agent file launches Claude and shows null in status; `programs.vexos-ai.opencodePackage` is now a removed option with a clear error. Handoffs X2/X4/X5 below. (original text:) If OpenCode is dropped: remove it from the panel, the module, the contract and the tests (map first). If kept: document the custom-agent route as accepted in docs/contract.md.
- [ ] **E4** Independent review of the send gate: composer.rs, check_usage, the source scans, and the helper's Send and Decide paths if Phase B ran. Use a fresh reviewer subagent with no prior context and ask it to try to find a path that sends text without a user click.
- [ ] **E5** Policy denies show "failed" because the core cannot tell them apart. Check whether the adapter exposes anything that lets the panel say "denied by policy". Do not guess. Report only.
- [ ] **E6 DECISION** Event::Usage (context-window line). Core change, additive, numbers only. Recommended: skip.

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
- [ ] **H2** Fix what H1 shows in the portable crates. Test-fixture problems (like the Unix `/tmp` working directory in core tests) are test fixes. Anything that changes how the core launches agents is a core change: stop and ask.
- [ ] **H3 DECISION** (A8) Native app or WSL installer; UI toolkit; .msi build and signing.
- [ ] **H4** (needs H3) Implement the platform boundary from `platform_boundary_spec.md`: traits for apply action, diagnostics, notifier, policy locations, agent launcher. Linux implementation first, no behaviour change.
- [ ] **H5** (needs H3) Windows policy install and repeat the spike escalation tests on Windows. The Linux sandbox harness does not work there.

---

## 11. Handoffs to other repos (Claude Code: you cannot edit these)

For each item, write the full prompt here (with the scope rule, verification and one commit message), tell the user which repo it belongs to, and wait for them to report back.

- [ ] **X1 VexPortal repo** Small cleanup: CLAUDE.md says no preflight script exists but one does; five older drift items (`_service-info`, `beszel`, `clean`, `rust-repair`, `static-ip`) need a look; the dialog text "Your sign-ins are kept either way" may be dead; one test (`reads_this_user_without_erroring`) has no assertion.
- [ ] **X2 vexos-nix repo** (needs A7) Add vexos-ai to the Daily workflow with a pre-push eval of a few variants, or a PR route.
- [ ] **X4 vexos-nix repo** (needs E3, same commit as the lock bump) Remove `opencodePackage` from `modules/ai.nix` (it is now a removed option and errors), and drop any OpenCode wording, tests or `opencode` package pin. Prompt still to be written.
- [ ] **X5 VexPortal repo** (needs E3) Remove `opencode` from the assistant catalog and the `pick` call. `vexos-ai pick opencode` now exits 2. Prompt still to be written.
- [ ] **X3 vexos-nix repo** (needs Phase B) If the Shell-native panel ships: the enabled-extensions handling and any new module option wiring.

---

## 12. Things the user deletes by hand

`~/.claude/plans/` old plan files. `~/.local/share/vexos/ai/claude-accounts/`. `/tmp/bats-run-*` and `/tmp/vexos-ai-spike-*`. `target/` scratch folders. `target/tools/dl/`. `~/.config/opencode/` (the old theme file `tui.json` and anything else OpenCode left).

---

## 13. Log (newest first; one line per item touched)

- 2026-10-31 F2 done: banner added to assistant_panel_spec.md.
- 2026-10-31 F3 done: scratch folders are ignored; nothing to remove but `target/`.
- 2026-10-31 E3 done: OpenCode removed (see item).
- 2026-10-31 A4 decided: require extension, no GTK fallback.
- 2026-10-31 A5 confirmed: end at close. A1 deferred by the user.
- 2026-10-31 A5 researched (Omarchy is on-demand, no resident process); awaiting final answer.
- 2026-10-31 A6 decided: drop OpenCode.
- 2026-10-31 A7 decided: add to Daily workflow with an eval gate.
- 2026-10-31 plan created.