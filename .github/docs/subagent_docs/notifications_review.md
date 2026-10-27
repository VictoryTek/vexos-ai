# Diagnosis notifications — review

**This is a self-review by the implementing agent, not an independent
review.** It is not marked approved. An independent review is still needed.

Spec: `notifications_spec.md`. Date: 2026-10-26.

## Files

- `crates/vexos-ai-viewmodel/src/redact.rs` (new)
- `crates/vexos-ai-viewmodel/src/diagnose.rs` (new)
- `crates/vexos-ai-viewmodel/src/notify.rs` (new)
- `crates/vexos-ai-viewmodel/src/lib.rs`
- `crates/vexos-ai-panel/src/notify.rs` (new)
- `crates/vexos-ai-panel/src/scan_tests.rs` (new)
- `crates/vexos-ai-panel/src/main.rs`
- `crates/vexos-ai-panel/src/rebuild.rs`
- `crates/vexos-ai-panel/Cargo.toml` (`vte4` feature `v0_72`; `Cargo.lock` unchanged)

Not touched: `nix/policy.nix`, `docs/contract.md`, CI, the extension,
`crates/vexos-ai-core`, `flake.nix`, `nix/*.nix`.

## Spec compliance

Met: draft-only flow, fixed instruction + fenced block, app-side read-only
collection with fixed argv, redaction and caps before display, no log text in
notifications, extension unchanged, no session started from a notification,
three triggers with a settings file and the requested defaults.

## Findings (self-review)

| # | Severity | Finding |
|---|---|---|
| 1 | Recommended | The never-send rule for the GTK path is checked by a source scan (`scan_tests.rs`), not a runtime test. It catches the mutation "notification calls `send_prompt`", but a rename could get around it. No headless GTK harness exists here. |
| 2 | Recommended | A placed draft **replaces** text the user had already typed in the input. |
| 3 | Minor | Enter sends (as before). The key controller runs in the capture phase, so with an input method (CJK) an Enter meant to commit a pre-edit sends instead. |
| 4 | Minor | The rebuild output range (`vadjustment().lower()` to the cursor row) has not been checked against a real VTE buffer. |
| 5 | Minor | Usernames outside `/home/<name>` (e.g. `[sudo] password for alice:`) are not redacted; a whole-word rule would mangle names like `nixos`. |
| 6 | Minor | Settings are read once per process; a change needs the panel restarted. |
| 7 | Info | With a watcher switched on, the panel keeps running after its window closes (app hold). It is not autostarted at login. |
| 8 | Info | The three new actions can be activated by any session-bus client. Targets are re-validated; the effect is only a draft. |

No critical findings from this pass.

## Build validation (exit codes)

Repo, in the devShell: `cargo build --all-targets` 0, `cargo test` 0 (core
3 + 43, view-model 84, panel 3), `cargo clippy --all-targets -- -D warnings`
0, `cargo fmt --all --check` 0.

On a copy of the tree (`target/tree-notify`; new files are untracked):
`nix build` 0, `checks.x86_64-linux.{policy,contract,core,panel,module,extension}`
each 0 (contract also with `--rebuild`, 118/118), `scripts/preflight.sh
path:…/target/tree-notify` 0, run twice (the second after a comment-only fix).

Mutation checks: 37/37 caught (after adding one test for a survivor).

## Score table (self-assessed)

| Category | Score | Grade |
|----------|-------|-------|
| Specification Compliance | 95% | A |
| Best Practices | 90% | A- |
| Functionality | 85% | B+ |
| Code Quality | 90% | A- |
| Security | 90% | A- |
| Performance | 90% | A- |
| Consistency | 95% | A |
| Build Success | 100% | A |

**Overall Grade: A- (92%)**. Functionality is lower because nothing on screen
was exercised.

Result: **PASS, self-review only. Not approved:** no independent review was done.
