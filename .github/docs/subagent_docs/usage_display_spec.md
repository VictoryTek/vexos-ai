# Assistant panel: always-visible usage display — spec

Status: **Phase 1 (research and specification)**, 2026-10-27. No code changed.
Builds on `assistant_panel_spec.md` ("usage header", step 3 of its section 7),
`assistant_panel_app_spec.md`, `platform_boundary_spec.md` and the spike
results (section 6, Terms).

Everything below comes from documentation, published schemas, the adapters'
published source at the pinned tags, and this repo's code. No agent was run,
and no data from this machine was read. Anything that only a running agent can
settle is listed under "Unverified" with exact steps (section 1.4).

## Summary

- ACP v1 (stable) has one usage message: the `usage_update` session update,
  carrying `used` (tokens currently in context), `size` (context window in
  tokens) and an optional cumulative `cost`. Per-turn token counts
  (`PromptResponse.usage`) exist only in the **unstable** schema.
- Both pinned adapters send `usage_update`. They also send the unstable
  `PromptResponse.usage`, but they mean different things by it, so the panel
  should not show it.
- The core drops `usage_update` today. Showing context use needs one additive
  core event, `Event::Usage { used, size }`: numbers only, no effect on any
  safety rule.
- **Subscription limits ("how much of my plan is left"): there is no
  documented, supported way** for a third-party UI to show them without the
  login token or an undocumented endpoint. **Do not build it.** This repo's
  CLI already does it the unsupported way (section 3.3). That is outside this
  task, but it conflicts with the rule set for the panel.
- Recommended display: one read-only line, for example
  `Context 53k of 200k (26%) · 4 tool calls · 1 approval waiting`. It shows
  `Context unknown` until the agent reports a figure, and drops to
  `26% · 1 waiting` in a narrow window.

## 1. What usage information ACP carries, and what each adapter sends

### 1.1 The protocol

Sources: `schema/v1/schema.json` and `schema/v1/schema.unstable.json` in
`agentclientprotocol/agent-client-protocol`, branch `main`, schema release
1.26.0 (2026-10-16). Read by streaming the published JSON through `jq`.
Also the session-usage RFD (agentclientprotocol.com/rfds/session-usage).

| Message | Schema | Fields | Meaning (schema wording) |
|---|---|---|---|
| `session/update` with `sessionUpdate: "usage_update"` (`UsageUpdate`) | **stable v1** | `used` uint64, required · `size` uint64, required · `cost` `{amount: double, currency: string}` or null, optional · `_meta` | "Tokens currently in context." · "Total context window size in tokens." · "Cumulative session cost (optional)." |
| `PromptResponse.usage` (`Usage`) | **unstable only** ("**UNSTABLE** This capability is not part of the spec yet, and may be removed or changed at any point.") | `totalTokens`, `inputTokens`, `outputTokens` required · `thoughtTokens`, `cachedReadTokens`, `cachedWriteTokens` optional | Described as "Token usage for this turn", yet the field descriptions say "across session" / "across all turns". The schema contradicts itself. |
| Stable `PromptResponse` | stable v1 | `stopReason`, `_meta` only | No usage. |

Further notes on the protocol:

- `usage_update` needs **no client capability** in the schema (unlike, for
  example, `compaction_update`, which needs `session.compaction`). The core
  does not have to advertise anything new to receive it.
- The RFD says `used` is a snapshot ("not a per-turn or running total"),
  cached tokens count towards it, and agents with no meaningful window size
  should not send the update. Clients derive `size - used` and `used / size`.
- The RFD gates the Rust SDK surface behind `unstable_session_usage`. That is
  irrelevant here: the core speaks raw JSON (`serde_json`) and does not use
  the SDK.
- **Not confirmed:** which schema release first carried `usage_update` as
  stable. The v1 changelog does not mention it, and the RFD's history says it
  was "Moved to Completed" on 2026-06-05. Both adapters below send it, which
  is what matters in practice.

### 1.2 `claude-agent-acp` 0.88.0 (the pinned, tested version)

Source: `zed-industries/claude-agent-acp`, tag `v0.88.0`, `src/acp-agent.ts`.
`package.json` confirms version 0.88.0, `@agentclientprotocol/sdk` 1.7.0 and
`@anthropic-ai/claude-agent-sdk` 0.3.293.

| What | Sent? | Where in source | Details |
|---|---|---|---|
| `usage_update` during streaming | **Yes** | around line 6643 | `{used, size}`, no `cost`. Sent when the running total of the latest assistant message changes. |
| `usage_update` at each turn result | **Yes** | around lines 6096-6112 | `{used, size, cost: {amount: total_cost_usd, currency: "USD"}}`, plus `_meta["_claude/origin"]` when set. |
| `usage_update` after compaction | **Yes** | around line 5201 | `used` = post-compaction tokens. |
| `usage_update` on an SDK `rate_limit_event` | **Yes**, only once a usage figure exists | around lines 7104-7112 | `_meta["_claude/rateLimit"] = rate_limit_info`. See section 3. |
| `_meta["_claude/model"]` on usage updates | Yes | `attachUsageModel`, around line 4154 | The model the figures belong to. |
| `PromptResponse.usage` (unstable) | **Yes** | `sessionUsage`/`turnOutcome`, around line 10141 | `{inputTokens, outputTokens, cachedReadTokens, cachedWriteTokens, totalTokens}`, **reset at each user turn** ("Accumulate usage into the user turn's tally"), main agent loop only. |
| `PromptResponse._meta.quota` | Yes | `turnQuotaMeta` | Adapter extension (codex-acp shape), including subagents. |

Caveats that matter for an honest display:

- **`size` can be a default, not a measurement.** `contextWindowSize` is
  seeded "from the per-model cache or the text heuristic
  (DEFAULT_CONTEXT_WINDOW when both miss)" (`DEFAULT_CONTEXT_WINDOW =
  200000`, line 367). It is then refined in the background and confirmed by
  each result's `modelUsage`. Updates sent **before the first turn result**
  may therefore carry 200000 for a model with a larger window. Nothing on the
  wire says which case applies (`contextWindowAuthoritative` is internal).
- **`cost` is an estimate.** Anthropic's cost-tracking docs
  (code.claude.com/docs/en/agent-sdk/cost-tracking): "The `total_cost_usd`
  and `costUSD` fields are client-side estimates, not authoritative billing
  data … Do not bill end users or trigger financial decisions from these
  fields." On a subscription login nothing is billed per token, so a dollar
  figure would mislead.
- The `_claude/*` `_meta` keys are not documented in the adapter's README.
  The README documents only the `_meta.jetbrains.air.*` keys. ACP itself says
  of `_meta`: "Implementations MUST NOT make assumptions about values at these
  keys."

### 1.3 `opencode acp` 1.19.0 (the pinned, tested version)

Source: `anomalyco/opencode`, tag `v1.19.0`, `packages/opencode/src/acp/`
(`usage.ts`, `service.ts`). `package.json` confirms version 1.19.0 and
`@agentclientprotocol/sdk` 0.21.0. The same tag is reachable under
`sst/opencode`.

| What | Sent? | Where | Details |
|---|---|---|---|
| `usage_update` | **Yes, once after each prompt turn** (also after a slash command or `/compact`), **not during streaming** | `service.ts` lines 542, 566, 588 → `usage.ts` `sendUpdate` | `used` = `input + cache.read + cache.write` of the **latest assistant message**. `size` = the provider's context limit for the model. `cost` = the sum of `cost` over every assistant message in the session, in USD. |
| No update when the limit is unknown | — | `usage.ts` `if (!size) return` | If the provider list has no context limit for the model, no `usage_update` is sent at all. |
| `PromptResponse.usage` (unstable) | **Yes** | `service.ts` `promptResponse` | `buildUsage(info)`: the tokens of **the turn's final assistant message only**. Earlier steps in a multi-step turn are not counted. |

### 1.4 Conclusions, and what is unverified

- **Show:** `used` and `size` from `usage_update`. Both adapters send them,
  both mean "tokens in context now" and "window size", and the message is
  stable ACP.
- **Do not show `PromptResponse.usage`.** It is unstable. Claude counts the
  whole user turn (main loop), OpenCode counts only the final message, and the
  schema contradicts itself on scope. One label cannot be honest for both.
- **Do not show `cost`.** For Claude it is a documented estimate. For
  OpenCode it is OpenCode's own computation (`0` for free models, presumably;
  not verified). It means nothing on a subscription.
- Expect OpenCode's context figure to update **only at turn end**, and
  Claude's during the turn.

**Unverified live checks.** Run these in the spike harness under its rules:
every permission request is rejected by `acp_client.py`, no real home, nothing
outside the namespace changes. I did not run them.

Preconditions (yours to decide): Claude needs a login. Under "no real home"
that means a throwaway config dir, and you sign in yourself through
Anthropic's own flow. I do not copy or link credentials. OpenCode uses the
free `opencode/big-pickle` model, as the spike did, so the test prompts go to
OpenCode Zen.

```sh
# From spike/acp, on the Linux host. Scratch dirs outside any repo.
S=$(mktemp -d /tmp/usage-check.XXXX)
mkdir -p "$S/home" "$S/work" "$S/claude-config"
export HOME="$S/home" XDG_CONFIG_HOME="$S/home/.config" \
       XDG_DATA_HOME="$S/home/.local/share" XDG_CACHE_HOME="$S/home/.cache" \
       XDG_STATE_HOME="$S/home/.local/state"

# U1: Claude. Sign in once, interactively, through Anthropic's own flow:
CLAUDE_CONFIG_DIR="$S/claude-config" claude auth login
printf '%s\n' 'Reply with the word ok.' 'Run exactly: touch x' | \
  CLAUDE_CONFIG_DIR="$S/claude-config" CLAUDE_AGENT_ACP_PROTOCOL=v1 ./sandbox.sh \
  ./acp_client.py --log out/u1-claude-usage.jsonl --cwd "$S/work" -- claude-agent-acp

# U2: OpenCode (free model, no login)
printf '%s\n' 'Reply with the word ok.' 'Run exactly: touch x' | ./sandbox.sh \
  ./acp_client.py --log out/u2-opencode-usage.jsonl --cwd "$S/work" -- opencode acp

# Read the results (the logs are JSON-RPC only; check them for secrets before sharing):
jq -c 'select(.params.update.sessionUpdate? == "usage_update") | .params.update' out/u1-claude-usage.jsonl
jq -c 'select(.result.stopReason?) | .result' out/u1-claude-usage.jsonl
# same two lines for out/u2-opencode-usage.jsonl
test ! -e "$S/work/x" && echo "PASS: touch was rejected"
rm -rf "$S"   # this deletes the throwaway login too
```

(If `acp_client.py` does not read one prompt per stdin line, send one prompt
per run. Check its `--help` first; I did not run it.)

What to record:

| # | Question | Expected from source |
|---|---|---|
| U1a | Claude sends `usage_update` during streaming and at turn end | yes, both |
| U1b | **The first update's `size`, compared with the model's real window.** Does a 1M-window model report 200000 before the first result? | possible (heuristic seed) |
| U1c | Does `_meta._claude/rateLimit` appear on a normal turn, or only near a limit? | only on an SDK `rate_limit_event` |
| U1d | `PromptResponse.usage` and `_meta.quota` present | yes |
| U2a | OpenCode sends exactly one `usage_update` per turn, after it ends | yes |
| U2b | `size` present for `opencode/big-pickle`, or no update at all | depends on the provider list |
| U2c | `cost.amount` for a free model | 0, presumably |
| U* | `x` was not created, and every permission request was rejected | must hold |

None of these change the recommendation. U1b decides only whether the tooltip
caveat in section 4.3 stays.

## 2. Does the core pass any of this up?

**No.** `crates/vexos-ai-core/src/session.rs` `on_update` (around line 428)
handles `agent_message_chunk`, `tool_call`/`tool_call_update`,
`current_mode_update` and `config_option_update`. `_ => {}` drops everything
else, `usage_update` included. `Session::prompt` returns
`result["stopReason"]` only (line 260), so `PromptResponse.usage` and `_meta`
are discarded too. `Event` has no usage variant. The only exhaustive `match`
on `Event` outside the core is `Conversation::apply`
(`crates/vexos-ai-viewmodel/src/conversation.rs:216`).

### 2.1 The additive core change (specified, not implemented)

One new variant, numbers only:

```rust
pub enum Event {
    // … existing variants unchanged …
    /// The agent's latest context report (ACP `usage_update`): tokens in
    /// context and the window size, as the agent sent them. For display only;
    /// it changes nothing in the session.
    Usage { used: u64, size: u64 },
}
```

In `Inner::on_update`, one new arm:

```rust
Some("usage_update") => {
    if let (Some(used), Some(size)) = (update["used"].as_u64(), update["size"].as_u64())
        && size > 0
    {
        let _ = self.events.send(Event::Usage { used, size });
    }
}
```

Rules:

- Emitted only for a `usage_update` whose `used` and `size` are both
  non-negative JSON integers and whose `size > 0`. A malformed or partial
  update emits **nothing**. That is not a safety event, so it does not reset
  or abort anything.
- `cost` and `_meta` are **never read**. No string from the agent crosses into
  this event.
- No new request, no new capability in `initialize`, no change to
  `Session::prompt`'s signature (the per-turn `usage` stays dropped).
- No change to mode pinning, the options filter, deny-by-default or
  `pinned`/`aborted`. The arm does not touch `mode_view`, `pending` or the
  connection.
- "Additive" at the protocol and behaviour level. In source, adding a variant
  to a public enum breaks exhaustive matches. The only one is
  `Conversation::apply`, which must gain an arm in the same change. That
  compile error is wanted, not something to avoid with `#[non_exhaustive]`.
- `lib.rs` module doc: one added sentence: "[`Event::Usage`] passes the
  agent's context figures up, read-only."

### 2.2 Fake-agent tests it needs (`crates/vexos-ai-core/tests/session.rs`)

Use the existing harness (`claude()`, `opencode()`, `Fake::update`, and a
permission request as the ordering barrier, as `modes_shown` does). Add a
helper `usage_shown(fake, events, barrier) -> Vec<(u64, u64)>`.

1. `usage_reports_are_passed_up_as_numbers`: both agents. `{used: 53000,
   size: 200000, cost: {amount: 0.04, currency: "USD"}, _meta: {...}}` gives
   exactly `[(53000, 200000)]`.
2. `malformed_usage_reports_are_dropped`: `used` missing, `size` missing,
   `size: 0`, negative, float (`1.5`), string (`"53000"`) and `null` each give
   nothing. A good one after them still comes through. Order is kept.
3. `usage_reports_change_nothing_else`: send a `usage_update` while a
   permission request is pending and after the pin. The request stays pending
   and is still answerable with `decide` only. `prompt_goes_through` still
   holds (no `NotPinned`, no reset request sent). The fake sees no message
   from the core caused by the update (`fake.seen` unchanged up to the
   barrier).
4. `usage_during_a_mode_reset_does_not_reopen_prompts`: a disallowed mode
   report, then a `usage_update` before the reset is acknowledged.
   `assert_refused` still holds.

Mutation checks, on a copy (`target/mut`), each reverted. Each must fail at
least one test:

| Mutation | Caught by |
|---|---|
| arm removed (`usage_update` ignored) | 1 |
| `size > 0` check removed | 2 |
| `used` and `size` swapped | 1 |
| arm also sets `pinned = true` | 4 |
| arm also calls `self.repin()` | 3 |
| `as_u64` replaced with `as_f64` cast (accepts floats) | 2 |

## 3. Subscription limits ("how much of my plan is left")

### 3.1 Finding

**There is no official, documented, supported way for a third-party UI to
show how much of a Claude subscription is left without reading the login
credentials or calling an undocumented endpoint.**

Evidence:

- ACP has no field for plan limits (section 1.1).
- The Agent SDK documents `SDKRateLimitEvent`
  (code.claude.com/docs/en/agent-sdk/typescript): `rate_limit_info: {status:
  "allowed" | "allowed_warning" | "rejected", resetsAt?, utilization?, …}`,
  "Emitted when the session encounters a rate limit". That is the SDK's own
  message, not ACP. `claude-agent-acp` 0.88.0 forwards it only inside
  `_meta["_claude/rateLimit"]` on a `usage_update`, a key its README does not
  document, which ACP says clients must not assume anything about. It arrives
  only when the SDK emits the event, not as a standing "N% left" reading. It
  is Claude-only; OpenCode has nothing similar.
- `claude-agent-acp` also turns Claude Code's own `/usage` command into
  markdown, via
  `query.usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()`
  (line 457). The name says it all, and it costs a prompt turn.
- Anthropic's terms, quoted in the spike results section 6: "Anthropic does
  not allow third party developers to offer claude.ai login **or rate limits**
  for their products" (Agent SDK overview), and "developers may not collect,
  store, or intermediate Claude.ai credentials or session tokens" (Legal and
  compliance). Whether a personal, self-hosted panel counts as a "third party
  developer" is spike open question 5, and still yours to decide. It does not
  change the technical answer above.

### 3.2 Recommendation

**Do not build a plan-remaining number in the panel.** The panel never reads
`.credentials.json`, the token, the CLI's usage cache
(`$XDG_CACHE_HOME/vexos/ai/usage-*.json`) or `vexos-ai status --json`'s
`usage` object, and never contacts any endpoint. It shows only what the agent
sends over its stdio.

**Not recommended either:** surfacing `_claude/rateLimit` as an "approaching
your limit" warning. It is an undocumented adapter key, Claude-only and
event-driven. Revisit only if `claude-agent-acp` documents it, or if ACP adds
a rate-limit message.

### 3.3 Conflicts found in this repo (reported, not changed: out of scope)

The rule "never read the login token, never call private endpoints" is
already broken outside the panel, by shipped code:

| Where | What it does |
|---|---|
| `bin/vexos-ai.sh:30` | `USAGE_ENDPOINT=https://api.anthropic.com/api/oauth/usage`. The comment at line 380 calls it "undocumented and rate-limited" |
| `bin/vexos-ai.sh:535-554` | reads `accessToken` from `.credentials.json`, sends it through `curl` with `anthropic-beta: oauth-2025-04-20` |
| `bin/vexos-ai.sh:452-470` | reads `refreshToken`/`accessToken`/`expiresAt`/`rateLimitTier` presence from the credentials for `signedIn` and `plan` |
| `nix/module.nix:165-178` | a systemd user service and timer run `vexos-ai usage-check` on a schedule |
| `docs/contract.md` "Usage probing", `tests/contract/usage.bats` | the public contract and its tests for the above |

Earlier specs also plan on this data: `assistant_panel_spec.md` section 5.5
(tile subtitle "<agent> · <N>% used", account rows with session and weekly
bars), assumption 2 in its section 10, and step 5 ("usage near limit"
notification).

These are a contract change (argv `usage`, `status --json` `usage`, exit
codes) and a module change. Each needs its own decision, spec and review.
This document does not decide them. The recommendation in 3.2 only keeps the
panel from depending on them.

## 4. What can be shown honestly

### 4.1 Data available

| Item | Source | Measured or derived | Honest? |
|---|---|---|---|
| Tokens in context, window size | `usage_update` via the new `Event::Usage` | as reported by the agent | Yes, labelled as the agent's figures. `size` may be Claude's default before the first result (U1b) |
| Context % | `used * 100 / size` (integer, floor) | arithmetic on reported figures | Yes |
| Tool calls this session | `Conversation`'s `tools` map (distinct `toolCallId`s) | counted by the panel | Yes |
| Approvals waiting | cards with `Answer::Pending` | counted by the panel | Yes, and the most useful item: a card can scroll out of view |
| Per-turn or session token totals | unstable `PromptResponse.usage` | inconsistent between adapters | **No**, dropped |
| Cost | `usage_update.cost` | estimate / means nothing on a subscription | **No**, dropped |
| Time since session start | the panel's own clock | measured | Honest, but needs a ticking timer for little value. **Dropped from the first slice** |
| Plan remaining | — | — | **No**, section 3 |

### 4.2 The element

A single read-only `gtk::Label` in a thin bar directly under the header bar
(`content.add_top_bar` after the header), visible whenever a conversation
exists. The header's own subtitle ("<agent> · <mode>") is unchanged. Dim
style (`dim-label`), `caption` size, no button, no hover action other than a
tooltip, not focusable.

Exact wording (the view-model produces these strings):

| State | Wide | Narrow |
|---|---|---|
| No usage report yet | `Context unknown · 0 tool calls · no approvals waiting` | `Context unknown` |
| Report received | `Context 53k of 200k (26%) · 4 tool calls · 1 approval waiting` | `26% · 1 waiting` |
| Several waiting | `… · 3 approvals waiting` | `26% · 3 waiting` |
| None waiting | `… · no approvals waiting` | `26%` |
| One tool call | `… · 1 tool call · …` | (not shown) |
| `used` > `size` | `Context 210k of 200k (105%) · …` | `105% · …` (shown as reported, not clamped) |
| Under 1% but > 0 | `Context 800 of 200k (<1%)` | `<1%` |
| `used` = 0 | `Context 0 of 200k (0%)` | `0%` |
| Session ended | last values unchanged (the subtitle already says "ended") | same |

Rules:

- Token figures: below 1,000 the exact number; below 1,000,000 floor to
  thousands with `k` (`53,999 → 53k`); from 1,000,000 floor to one decimal
  with `M` (`1,049,999 → 1.0M`). Rounding is always down, so the display
  never overstates the window or understates use by more than the unit. The
  percentage is a floor, with `<1%` for a non-zero value under 1.
- **"unknown" means no report yet.** The panel never fills in a default
  window, never estimates tokens from text length, and never carries a figure
  over from a previous session (a new `Conversation` starts at "unknown").
- When narrow, the count of waiting approvals stays and the tool calls go:
  waiting approvals need action.

### 4.3 Tooltip (same in wide and narrow)

`Context figures are reported by the agent. The window size may be the
agent's default until its first reply finishes.`

When no report has arrived: `The agent has not reported its context use yet.
OpenCode reports it after each reply.`

Drop the second sentence of the first tooltip if U1b shows the first `size`
is always the real window.

### 4.4 Narrow window

`adw::Breakpoint` on the `adw::ApplicationWindow` (libadwaita ≥ 1.4; the lock
has 1.9.3. Docs: "Breakpoints can be used within AdwWindow,
AdwApplicationWindow, AdwDialog or AdwBreakpointBin", condition syntax
`max-width: 400px`). Condition `max-width: 480sp`. When it applies, the panel
shows `UsageLine::narrow()` instead of `wide()`. Simplest wiring: the
breakpoint's `apply`/`unapply` signals flip a `Cell<bool>` and call
`refresh()`. The label also gets `ellipsize = End` as a last resort, so it
never forces the window wider. **To verify at implementation:** that the
`libadwaita` 0.9 crate exposes `Breakpoint::new`, `BreakpointCondition::parse`
and `connect_apply`/`connect_unapply` under the enabled feature level (only
the C docs were read here).

## 5. Safety review

| Requirement | Holds? | How |
|---|---|---|
| Read-only: no control changes mode, agent or approvals | **Yes, by construction** | The element is a `gtk::Label`: no `connect_clicked`, no gesture, no menu, not focusable. `Usage`/`UsageLine` (section 7) have no method taking `Decider`, `Session` or `&mut Conversation`, and nothing that returns a `Decision`. The core event carries two `u64`s and the core reads nothing back. A source-scan test (section 7.3) asserts the panel code builds no button or action from the usage line. |
| Never answers a permission request | **Yes** | `Conversation::apply(Event::Usage{..})` only stores numbers. The existing test that `apply` never calls the decider covers the new arm once it is included in the event list. |
| Does not log usage with prompt text | **Yes** | The panel and view-model contain no logging (no `println!`/`eprintln!`/`log`/`tracing`; checked by grep) and write no files for conversations. The usage numbers are never written anywhere. One existing caveat, unchanged: the agent's **own** stderr is inherited (`session.rs:171`), so whatever the adapter logs goes to the panel's stderr. That is the agent's output, not the panel's, and this change does not alter it. |
| Sends nothing anywhere | **Yes** | No network code in the panel or view-model. The figures arrive on the agent's existing stdio and go only to a label. The core sends nothing in reply to a `usage_update` (core test 3). |
| Does not read credentials or call private endpoints | **Yes** | No file reads are added. Source-scan test (section 7.3) forbids `credentials`, `oauth`, `api.anthropic.com`, `usage-`, `status --json` and `Command::new` in the usage code. |
| No safety rule weakened | **Yes** | Section 2.1: the arm touches no pinning, pending or abort state. Core tests 3 and 4 plus mutations. |
| Agent text never becomes UI | **Yes** | Only integers cross. `_meta` and `cost` are not parsed. |

Skipped in this task, per the scope rule: no `reviewer` subagent was run (the
deliverable is a design document, and the scope rule overrides CLAUDE.md's
Tier 3 review). No checks were run.

## 6. Platform fit

| Part | Crate | Portable? |
|---|---|---|
| `Event::Usage` parsing | `vexos-ai-core` | **Yes.** `serde_json` only; no platform API |
| `Usage` state, counting, number formatting, wide/narrow strings, tooltip text | `vexos-ai-viewmodel` (new `usage.rs`) | **Yes.** Pure logic; no GTK, systemd, `/etc` or clock. Tested headless on the Windows CI job too |
| Breakpoint, label, CSS classes, where the bar sits | `vexos-ai-panel` | **Display-only** (GTK/libadwaita). A future Windows front end calls the same `UsageLine::wide()`/`narrow()` and chooses its own width threshold |

No new Linux assumption is added. The display never reads `$XDG_*`, never
runs a command and never looks at the filesystem.

## 7. View-model types and headless tests

### 7.1 Types (`crates/vexos-ai-viewmodel/src/usage.rs`, new)

```rust
/// What the agent last reported about its context window. `None` until the
/// first report. Set only from `Event::Usage`; there is no other setter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context { pub used: u64, pub size: u64 }

/// The always-visible usage line, built from a conversation. Read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageLine {
    pub context: Option<Context>,
    pub tool_calls: usize,
    pub waiting: usize,
}

impl UsageLine {
    pub fn wide(&self) -> String;      // section 4.2 "Wide"
    pub fn narrow(&self) -> String;    // section 4.2 "Narrow"
    pub fn tooltip(&self) -> &'static str;
}

pub fn format_tokens(n: u64) -> String;          // 999, 53k, 1.0M
pub fn context_percent(c: Context) -> String;    // "26%", "<1%", "105%"
```

`Conversation` gains a private `context: Option<Context>` and
`pub fn usage(&self) -> UsageLine`. `tool_calls` is `self.tools.len()`;
`waiting` counts `Item::Approval` with `answer == Answer::Pending`.
`apply(Event::Usage { used, size })` sets `context` and returns
`Vec::new()` (no transcript item changes). Export `UsageLine`, `Context`,
`format_tokens` and `context_percent` from `lib.rs`. Add one line to the
crate doc: "The usage line shows only what the agent reported and what the
panel counted; it has no controls."

### 7.2 Headless tests (view-model)

1. `usage_is_unknown_until_reported`: a new `Conversation` gives
   `context == None` and the exact "unknown" wide and narrow strings.
2. `a_report_is_shown_exactly`: `Event::Usage{53000, 200000}` gives the exact
   wide string `Context 53k of 200k (26%) · 0 tool calls · no approvals
   waiting`.
3. `the_latest_report_wins`: two reports; the second is shown.
4. `format_tokens_rounds_down`: 0, 999, 1000, 53_999, 999_999, 1_000_000,
   1_049_999, 1_999_999.
5. `percent_edges`: 0, `<1%`, exactly 1%, 99.9% shown as 99%, 100%, over
   100% (not clamped).
6. `counts_tool_calls_by_id`: two updates for one id plus a second id gives
   2. Singular and plural wording.
7. `counts_only_pending_approvals`: approve one, deny one, one timed out
   (`Event::TimedOut`), one pending gives `1 approval waiting`. After
   `close()`, 0.
8. `narrow_keeps_waiting_and_drops_tool_calls`: exact narrow strings for 0,
   1 and 3 waiting, and for unknown context.
9. `usage_never_decides`: extend the existing "apply never calls the
   decider" test's event list with `Event::Usage`.
10. `a_new_conversation_starts_unknown`: no figure carries over.

Mutation checks (on a copy, each reverted; each must fail a test): floor
replaced by round in `format_tokens`; `<1%` branch removed; the clamp
`min(100)` added; `waiting` counts all cards; `tool_calls` counts updates
instead of ids; `apply` ignores `Event::Usage`; narrow drops the waiting
count.

### 7.3 Panel (GTK, not run headless)

- Extend `crates/vexos-ai-panel/src/scan_tests.rs`. The usage wiring (a new
  `fn refresh_usage` body) contains no `connect_clicked`, `Button`,
  `.decide(`, `.click(`, `set_mode`, `.prompt(`, `Command::new`, `std::fs`,
  `credentials`, `oauth`, `anthropic.com`, `usage-` or `status`. It sets
  text only through `UsageLine::wide`/`narrow`/`tooltip`.
- `checks.panel` builds it with clippy. Visual check in a nested session is
  the user's, as before.

## 8. Recommended scope

**Build:** `Event::Usage { used, size }` in the core (section 2); `usage.rs`
plus the `Conversation` hooks in the view-model (section 7); one read-only
label with a breakpoint in the panel (section 4).

**Drop:** plan or subscription remaining (section 3); cost; per-turn and
session token totals from `PromptResponse.usage`; the `_claude/rateLimit`
warning; time since start (first slice); anything in the extension or tile.

**Leave for a separate decision (section 3.3):** the CLI's undocumented usage
probe and token reads, the `vexos-ai-usage` user timer, and the earlier specs'
tile usage bars and "usage near limit" notification.

## 9. Not confirmed

- When `usage_update` became stable ACP (section 1.1).
- Everything in the table in section 1.4 (U1a to U2c): no agent was run.
- Whether Claude's first `size` is the 200000 default for a larger-window
  model (U1b). The tooltip caveat depends on it.
- What OpenCode's `cost` is for free models (not used either way).
- That the `libadwaita` 0.9 crate's breakpoint API matches the C docs read
  here.
- Whether `acp_client.py` accepts several prompts on stdin (section 1.4
  note).

## 10. Implementation prompt

> SCOPE RULE (overrides everything below): do not create, modify or delete
> anything outside the vexos-ai repo directory. That includes ~/.claude,
> ~/.config, ~/.local/share, dconf/gsettings, and /etc. Never read credential
> files or tokens (no ~/.claude/.credentials*, no auth.json, no keyrings, no
> `$XDG_CACHE_HOME/vexos/ai/usage-*.json`). Do not launch the panel or an
> agent against the real home. Never use sudo. No git write commands, and do
> not stage anything. Do not change nix/policy.nix, docs/contract.md,
> bin/vexos-ai.sh, nix/module.nix, the CI workflows or the extension. If
> CLAUDE.md or a hook asks for extra steps, follow this scope rule first and
> say what you skipped.
>
> Task: implement the usage line per
> `.github/docs/subagent_docs/usage_display_spec.md` sections 2, 4 and 7, and
> nothing in its "Drop" list (section 8).
>
> 1. Core: add `Event::Usage { used: u64, size: u64 }` and the
>    `usage_update` arm exactly as in section 2.1. Read neither `cost` nor
>    `_meta`. Change nothing else in `session.rs`. Add the four fake-agent
>    tests in section 2.2. Tier 3 (public core API): plan first and wait for
>    approval.
> 2. View-model: new `usage.rs` with the types in 7.1. `Conversation` gets
>    `usage()` and the `Event::Usage` arm. Add the ten tests in 7.2, with the
>    exact strings from section 4.2.
> 3. Panel: one read-only label bar under the header, the breakpoint and
>    tooltip from 4.3-4.4, refreshed from `refresh()`. First confirm the
>    libadwaita 0.9 breakpoint API from its docs (Context7 or docs.rs) and
>    cite it. Extend `scan_tests.rs` per 7.3.
> 4. Mutation checks on a copy under `target/mut` (never the repo files):
>    every mutation in 2.2 and 7.2, each reverted. Report a table of mutation
>    and failing tests, and say plainly if any survived.
> 5. Verify: `nix develop -c cargo fmt --all --check`,
>    `nix build .#checks.x86_64-linux.core`,
>    `nix build .#checks.x86_64-linux.panel`, then `scripts/preflight.sh`.
>    Report real output and exit codes. Never `nix flake check`.
> 6. Independent review by the `reviewer` subagent, given this spec and the
>    changed files. Fix in-scope findings and re-run.
> 7. Do not run the live checks U1/U2 yourself. If they have been run, read
>    `spike/acp/out/u1-*.jsonl` and `u2-*.jsonl` only for `usage_update` and
>    `stopReason` lines, and adjust the tooltip (4.3) to match. Otherwise
>    keep the caveat.
>
> Honest reporting: never write "passes" for something that did not run.
> List anything unverified in one line each. End with one commit message in
> the project format, with check lines only for checks that ran and passed.
