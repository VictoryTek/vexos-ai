# Assistant panel app, first slice — review

Phase 3, 2026-10-25. Spec: `assistant_panel_app_spec.md`.

## Files reviewed

Commit A (Step 0 + core event): `crates/vexos-ai-core/src/{lib,rpc,session}.rs`,
`crates/vexos-ai-core/tests/session.rs`, `crates/vexos-ai-core/examples/smoke.rs`,
`.github/docs/subagent_docs/assistant_panel_spike_results.md`.

Commit B (Steps 1 and 2): `Cargo.toml`, `Cargo.lock`, `flake.nix`, `nix/panel.nix`,
`crates/vexos-ai-viewmodel/**`, `crates/vexos-ai-panel/**`, the spec and this review.

## Findings

| # | Severity | Where | Finding |
|---|---|---|---|
| 1 | RECOMMENDED (bug, not safety) | `crates/vexos-ai-panel/src/main.rs` `Ui::refresh` | Returns before updating the rebuild button when no conversation exists yet, so on the start page the button stays clickable during the confirmation and while a rebuild runs. `Rebuild::request` refuses the click, so nothing runs twice, but the UI is wrong. |
| 2 | MINOR | `crates/vexos-ai-panel/src/rebuild.rs` `finished` | Uses the full path `vexos_ai_viewmodel::Rebuild` where the file imports from the crate already. |
| 3 | NOTE | `flake.nix` `checks.panel` | CI's `nix flake check` now also compiles the GTK crate (binary GTK libs from the cache; the Rust crates compile). Longer CI, no new risk. |
| 4 | NOTE | core `handshake` | Pre-existing race (see spike results, "Core mode event"): `pinned` is set unconditionally after the first pin. Out of scope (safety behaviour); reported, not fixed. |

Safety review against the spec:

- The core change is additive: the `pinned`, `mode_reports` and abort paths
  are unchanged. `ModeView` only gates `Event::Mode`. The one lock is never held
  across an `.await`.
- The view-model has no path from `apply` to `Decider`. `close` sends only
  `DenyOnce`. `buttons` comes only from the core's `choices`. There is no
  mode setter.
- The panel calls `Session::decide` only from a button's `clicked` handler
  (through `Conversation::click`) and from `close`. It never calls a mode API
  (the core has none).
- Rebuild: the argv and directory are constants in `vexos-ai-viewmodel`, and
  `RebuildCommand` can't be built outside `Rebuild::confirm`. The
  conversation code has no reference to `Rebuild`. The terminal widget is
  never read. The child is spawned without a shell, and input is disabled on
  exit or on a spawn failure.

## Build and test results (commands run, exit codes)

| Command | Exit |
|---|---|
| `cargo build --all-targets` (devShell) | 0 |
| `cargo test` (devShell): core 3 + 40, view-model 29, panel 0 | 0 |
| `cargo clippy --all-targets -- -D warnings` (devShell) | 0 |
| `cargo fmt --check` (devShell) | 0 after `cargo fmt` (first run: 1, one wrapped `format!`) |
| `nix build path:<copy>#default` | 0 |
| `nix build path:<copy>#checks.x86_64-linux.policy` (`--rebuild`) | 0 |
| `nix build path:<copy>#checks.x86_64-linux.contract` (`--rebuild`; 118/118 ok) | 0 |
| `nix build path:<copy>#checks.x86_64-linux.core` (3 + 40 + 29 tests, clippy) | 0 |
| `nix build path:<copy>#checks.x86_64-linux.panel` | 0 |
| `nix build path:<copy>#vexos-ai-panel` | 0 |
| `nix eval path:<copy>#nixosModules.default --apply 'm: builtins.typeOf m'` | 0 (`"lambda"`) |

`<copy>` is an rsync of the working tree without `target/`, `result*`,
`spike/acp/out` and `.git`. The new files are untracked, so a `git+file`
flake build would not see them, and this workflow does not stage files.

Mutation checks: core 7/7 caught; view-model 18/18 caught (sources restored,
`cmp`-checked).

## Scores

| Category | Score | Grade |
|----------|-------|-------|
| Specification Compliance | 95% | A |
| Best Practices | 92% | A- |
| Functionality | 90% | A- |
| Code Quality | 92% | A- |
| Security | 96% | A |
| Performance | 95% | A |
| Consistency | 93% | A |
| Build Success | 100% | A+ |

**Overall Grade: A- (94%)**

## Result: NEEDS_REFINEMENT

Finding 1 is a real defect in shipped UI code, so it is fixed before
preflight (cycle 1). Finding 2 is fixed with it.
