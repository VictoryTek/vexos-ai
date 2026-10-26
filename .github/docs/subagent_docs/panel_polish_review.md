# Panel polish — review

2026-10-26. Spec: `panel_polish_spec.md`. This is a **self-review** by the
agent that wrote the change, not an independent one. Nobody else reviewed it.

## Spec compliance

| Item | Done | Note |
|---|---|---|
| 1. Markdown | Yes | `viewmodel/src/markdown.rs`; the window inserts runs as literal text with tags |
| 2. "denied" | Yes, with a limit | policy denies can't be told apart from failures (no request, wording dropped by the core); they say "failed" |
| 3. "Timed out, denied" | Yes | additive `Event::TimedOut(u64)` in the core, sent after the deny |
| 4. `verify-host.sh` | Yes | read-only; shellchecked; **not run** |

## Findings

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | Test bug | `answered_or_cancelled_requests_are_never_reported_timed_out` failed on unmutated code: its fake-agent timeout (30 s, paused time) elapsed while the test waited for stray events, so the core correctly reported a timeout | Fixed: the test now checks the stray-event wait is empty only after each answer, and asserts the approve/deny outcome first |
| 2 | Note | First mutation run was invalid (`-q` hid test names; one mutant didn't compile) | Harness fixed, all mutations re-run |
| 3 | Note | A reply is re-parsed and its widget rebuilt on every text chunk. Fine for short replies; untested on long ones | Unchanged |
| 4 | Note | "denied" also shows for a call whose card was cancelled or closed with the session, not only a click on Deny | Intended: it was not approved |
| 5 | Note | No code in the safety path changed: the response to the agent is unchanged; the event is sent after it | — |

## Safety review of the core change

- The deny response (`pending.options.outcome(..).unwrap_or_else(deny)`) is
  untouched. `TimedOut` is sent after `respond`, only when `timeout()` elapsed
  and this task took the pending entry.
- Existing timeout tests pass unchanged. Two new fake-agent tests; both
  mutations of the flag are caught.
- Nothing in the window answers a request. `Conversation::apply` still has no
  `Decider`.

## Verification (exit codes in `assistant_panel_spike_results.md`)

Build, tests, clippy, fmt, `nix build`, `checks.{policy,contract,core,panel,module,extension}`
and `scripts/preflight.sh` all exited 0 on the final code. The nix checks ran
on a copy of the working tree (`target/tree-polish2`), because the new files
are untracked; it matched the repo file for file.

## Scores (self-assessed)

| Category | Score | Grade |
|----------|-------|-------|
| Specification Compliance | 92% | A |
| Best Practices | 90% | A |
| Functionality | 88% | B+ |
| Code Quality | 90% | A |
| Security | 92% | A |
| Performance | 85% | B |
| Consistency | 92% | A |
| Build Success | 100% | A+ |

Functionality and Performance are held down because nothing was rendered on
screen. **Overall Grade: A- (91%)**. Result: **PASS**, self-reviewed.
