# Draft flow hardening — review

**This is a self-review by the implementing agent, not an independent
review.** It is not marked approved.

Date: 2026-10-26. Follows `notifications_review.md` findings 1 and 2.

## Files

- `crates/vexos-ai-viewmodel/src/composer.rs` (new)
- `crates/vexos-ai-viewmodel/src/conversation.rs` (`send` takes `&Outgoing`)
- `crates/vexos-ai-viewmodel/src/diagnose.rs` (`HeldDraft` removed, replaced by `Composer`)
- `crates/vexos-ai-viewmodel/src/lib.rs`
- `crates/vexos-ai-panel/src/main.rs`
- `crates/vexos-ai-panel/src/scan_tests.rs`
- `flake.nix` (one `checks.module` case)

Not touched: `nix/policy.nix`, `docs/contract.md`, CI, the extension,
`crates/vexos-ai-core`, `nix/module.nix`, `nix/panel.nix`.

## Findings (self-review)

| # | Severity | Finding |
|---|---|---|
| 1 | Fixed during review | Discard after "Add below" cleared the user's own text, and turned an earlier draft into plain text. Discard now restores the input as it was, draft state included. Tests and two mutations added. |
| 2 | Recommended | Enter on an unreviewed draft does nothing and gives no feedback beyond the banner text. |
| 3 | Minor | Any edit, even one space or Shift+Enter, makes Enter send the draft. This is what the task asked for; the banner stays until it is sent or discarded. |
| 4 | Minor | Clearing the input by hand ends the draft (nothing is left to review). |
| 5 | Minor | A message the core refused comes back as typed text. If it was a draft, the banner does not return; the user had already clicked Send once. |
| 6 | Info | The wiring check (only Send/Enter call `submit`, and `session.prompt` only sends what `submit` let out) is still a source scan: core's `Session::prompt(&str)` is public, and the GTK handlers can't run headless. |
| 7 | Carried over | The IME/Enter capture-phase issue (`notifications_review.md` #3) is unchanged. |

## Build validation

See the "Draft flow hardening" section of `assistant_panel_spike_results.md`.
Every command listed there exited 0. The Nix checks ran on a tree copy.

## Score table (self-assessed)

| Category | Score | Grade |
|----------|-------|-------|
| Specification Compliance | 95% | A |
| Best Practices | 90% | A- |
| Functionality | 85% | B+ |
| Code Quality | 90% | A- |
| Security | 95% | A |
| Performance | 95% | A |
| Consistency | 95% | A |
| Build Success | 100% | A |

**Overall Grade: A- (93%)**. Functionality is lower because nothing was
seen on screen.

Result: **PASS, self-review only. Not approved:** no independent review was done.
