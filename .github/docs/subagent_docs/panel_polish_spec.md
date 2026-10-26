# Panel polish — spec

Status: Phase 1, 2026-10-26. Builds on `assistant_panel_spec.md`,
`assistant_panel_app_spec.md` and the "Extension" results. No change to
`nix/policy.nix`, `docs/contract.md`, CI, or the core's safety behaviour
(pinning, allow-list, options filter, deny-by-default).

## 1. Request

1. Agent replies rendered as markdown; agent output is untrusted.
2. A tool call whose approval was denied says "denied", not "failed".
3. A request the core's timeout denied says "Timed out, denied".
4. `scripts/verify-host.sh`: a missing `/etc/<rel>` with an existing
   `/etc/static/<rel>` says the shell may be sandboxed with its own `/etc`.

## 2. Current state

- `Item::Agent(String)` is shown as a plain `gtk::Label` (no markup).
- Tool cards show the raw ACP status (`pending`, `in_progress`, `completed`,
  `failed`). Both a denied and a broken call arrive as `failed`.
- `PermissionRequest.tool_call_id` links a request to its tool call; the
  view-model drops it today.
- The core denies an unanswered request after `permission_timeout` (600 s)
  and tells the UI nothing: the card keeps its buttons, and a later click gets
  `UnknownRequest` and shows "No longer pending".
- A policy deny (managed deny rule) sends no permission request. The agent
  reports `failed` with agent-specific wording in the tool call's content,
  which the core does not pass up.

## 3. Design

### 3.1 Markdown (`crates/vexos-ai-viewmodel/src/markdown.rs`)

`markdown(&str) -> Vec<Run>`; a `Run` is literal text plus a `Style`
(bold, italic, code, heading). Parsed with `pulldown-cmark` 0.13,
`default-features = false` (no HTML renderer), `Options::empty()`.

- The window inserts each run into a `GtkTextBuffer` with
  `insert_with_tags_by_name`: plain text, never markup. No Pango markup is
  ever built, so there is nothing to escape and nothing to inject.
- Raw HTML (block and inline) is shown as literal text.
- Links and images are text: the label, then the real target in brackets.
  Never clickable, never loaded.
- Lists: `• ` or `N. `, two spaces per level, indent capped at 8 levels.
- Only `Item::Agent` is rendered this way. Approval cards (command, plan,
  diff) and user messages stay literal.

### 3.2 Denied wording (view-model only)

`ApprovalCard` keeps `tool_call`. `Conversation::tool_status(&ToolCard)`
says `denied` when the status is `failed` and the latest approval for that
tool call was answered without approval (denied, timed out, withdrawn,
closed). Everything else shows the status as is. A change to a card's answer
also reports the linked tool card's index for redraw.

**Policy denies are not distinguishable** with what the core passes up (no
request, same `failed`, wording only in content the core drops). They still
say `failed`. Not guessed.

### 3.3 Timed-out approvals (additive core event)

- Core: `Event::TimedOut(u64)`, sent by `on_permission` **after** it answered
  the request with the deny outcome, only when the timeout elapsed and that
  task took the pending entry. The response sent to the agent is unchanged.
- View-model: `Answer::TimedOut`; the event sets it on the card whatever the
  card said, since the core's answer was a deny. The window shows
  "Timed out, denied." and no buttons. Events are read while the window is
  hidden, so it applies there too.

### 3.4 `verify-host.sh`

In `check_policy`, when `/etc/<rel>` is missing and `/etc/static/<rel>`
exists, fail with "missing, but /etc/static/<rel> exists: this shell may be
sandboxed with its own /etc". Read-only, as before.

## 4. Dependencies

`pulldown-cmark` 0.13 (Context7 `/pulldown-cmark/pulldown-cmark`: `Event`,
`Tag`, `TagEnd`; `default-features = false`). Pure Rust, vendored by
`cargoLock.lockFile`; no new Nix input.

## 5. Verification (safe commands only)

In the devShell: `cargo build --all-targets`, `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Then
`nix build`, `checks.x86_64-linux.{policy,contract,core,panel,module,extension}`,
`scripts/preflight.sh`, shellcheck of `scripts/verify-host.sh`. Mutation
checks for the markdown inertness rules, the denied wording, the timed-out
card, and the core event. `verify-host.sh` itself is not run. Never
`nix flake check`.

## 6. Risks

| Risk | Mitigation |
|---|---|
| Markup injection | No markup: literal insert with tags; tests on hostile input |
| Deceptive links | The real target is always shown; nothing is clickable |
| Core change affecting safety | Event sent after the unchanged deny response; existing timeout tests unchanged |
| Re-parsing long replies on every chunk | Accepted; noted as untested for long conversations |
