# Diagnosis notifications — spec

Status: Phase 1, 2026-10-26. Builds on `assistant_panel_spec.md` (section 7,
step 5: GNotification actions) and `assistant_panel_app_spec.md`. No change
to `nix/policy.nix`, `docs/contract.md`, CI, the extension, or the core.

> **Update, 2026-10-29: the failed-rebuild trigger was removed** together with
> the panel's "Run just rebuild" button it depended on (the notification fired
> when the app's own terminal exited with a failure). Gone: the `rebuild-failed`
> setting (an old line for it in `notifications.conf` is now an unknown key and
> is ignored), the `app.diagnose-rebuild` action, `Subject::Rebuild` and the
> "rebuild output is no longer available" draft text. Kept: failed systemd
> units, coredumps, the draft banner and composer, and the settings file.
> Applying changes stays a terminal action: `just rebuild`. Sections below that
> describe the rebuild trigger are history.

## 1. Request

Notifications for a failed rebuild, a failed systemd unit and a coredump of
the user's own process. A click opens the panel with a **draft** prompt in the
input; nothing is sent until the user clicks Send. Collected text is
untrusted, redacted, capped and fenced. Each trigger is switchable; only the
rebuild trigger is on by default.

## 2. Conflicts with earlier specs (the request's rules win)

| Earlier text | Now |
|---|---|
| `assistant_panel_app_spec.md` §3: the rebuild terminal's output "never reaches the conversation" | It still never reaches it **by itself**. Its last lines can appear in a draft the user reviews and sends. Fetched from the VTE buffer only at click time. |
| `docs/spec.md` §3.1: `vexos-ai notify rebuild-failed` called from vexos-nix's `just rebuild`, actions `diagnose-crash(pid)` | Not built: it would be a contract change (forbidden here). Only the app's own rebuild terminal triggers. Actions are `diagnose-rebuild`, `diagnose-unit(s)`, `diagnose-coredump(s)`. |
| `assistant_panel_spec.md` §7 step 5: usage, welcome, search provider | Out of scope for this task. |

## 3. Current state

- `Rebuild` state machine (view-model) and a VTE window (panel), fixed argv
  `just rebuild` in `/etc/nixos`. The rebuild window is destroyed on close.
- The panel is a `GtkApplication` with one action, `toggle`; D-Bus activatable
  (service file, desktop file). The composer is a single-line `gtk::Entry`,
  which cannot show a multi-line draft faithfully.
- No `regex` in `Cargo.lock`; `vte4` 0.10 has `text_range_format` behind the
  `v0_72` feature (nixpkgs vte-gtk4 is 0.84).

## 4. Design

### 4.1 View-model (no GTK), new modules

- `diagnose.rs`
  - `excerpt(raw: &[u8]) -> Excerpt`: lossy UTF-8; strip ANSI escapes; replace
    other control and bidi-override characters; redact; cut each line at 500
    characters; keep the **last** 200 lines within 16 KiB. Reports kept/total
    lines, lines cut, values redacted, characters replaced.
  - `redact`: private key blocks (also an orphan END), `Authorization:`
    headers, `Bearer|Basic|token <value>`, `key=value`/`key: value` for
    password/secret/token/api key style keys, known token prefixes (`sk-`,
    `ghp_`, `github_pat_`, `glpat-`, `xox?-`, `AKIA`, `AIza`), URL userinfo
    passwords, long high-entropy runs (≥ 32 chars, upper+lower+digit,
    entropy ≥ 4 bits/char; SRI hashes `sha256-…` kept, nix base32 and hex
    never qualify), `/home/<name>` → `/home/[user]`. Hand-written scanners, no
    new dependency.
  - `draft(subject, log) -> String`: a request line from a fixed template with
    only validated identifiers, then the fixed `INSTRUCTION`, then a note made
    from counts only, then `BEGIN`, every line prefixed `| ` with `<<<`/`>>>`
    neutralised, then `END`. Missing logs get a fixed reason (no journal
    access, output gone, command failed) instead of a block.
  - `UnitName::parse`: 1–255 chars of `[A-Za-z0-9:_.@-]` plus `\xHH`
    escapes, no leading `-` or `.`, a known unit suffix. `journal_argv`:
    `journalctl --no-pager --output=short-iso --no-hostname --lines=200
    --unit=<name>` (no `--quiet`: its access hint is how a missing permission
    is detected). `coredump_argv(pid, uid)`: `coredumpctl info --no-pager
    COREDUMP_PID=<pid> COREDUMP_UID=<uid>`.
  - `journal_log` / `coredump_log`: classify output (`not seeing messages from
    other users`, `insufficient permissions`, `No coredumps found`).
  - `HeldDraft`: holds a draft until the conversation can send; releasing it
    gives the text for the input, nothing else.
- `notify.rs`
  - `Settings::parse` of `$XDG_CONFIG_HOME/vexos-ai/notifications.conf`
    (`rebuild-failed`, `failed-units`, `coredumps` = `true|false`). Missing
    file or bad value: the default (on, off, off).
  - `Limiter`: per-key cooldown 1 h, global cap 3 per 10 min.
  - `FailedUnits::observe(names, now)`: first call is a silent baseline; then
    each valid name newly in the failed list, if the limiter allows. A unit
    that recovers and fails again is new again (still under its cooldown).
  - `Crashes::observe(file_name, now)`: parses the core file name, own uid
    only, each file once, limited per command name.

### 4.2 Panel

- `notify.rs`: sends `gio::Notification`s (fixed title, body = validated
  name or app text, default action with a validated target) and handles the
  three actions: present the window, collect read-only with
  `gio::Subprocess::newv` (argv from the view-model, 10 s timeout), build the
  draft, hold it. **No call to `send_prompt` or `Session`.**
- Failed units: every 60 s, `ListUnitsFiltered(["failed"])` on the system bus
  (read-only, no subprocess). Coredumps: a `gio::FileMonitor` on
  `/var/lib/systemd/coredump`. Started only when switched on; then the app
  holds itself so watching continues after the window closes (it is not
  autostarted at login).
- Rebuild: on a non-zero exit, notify. The rebuild window hides on close and
  is kept until the next run, so its buffer can be read at click time.
- Composer becomes a multi-line `TextView` (Enter sends as before,
  Shift+Enter newline). A placed draft does not take focus.
- On the agent-choice page a line says a draft is waiting. (Superseded 2026-10-28: there is no choice page; the note shows while the session starts.)

## 5. Tests (headless)

Redaction and capping fixtures (keys, bearer, key blocks, URL passwords,
huge lines, binary junk), mutation-checked on a tree copy; injection fixtures
(the instruction is constant, content stays inside one block, a forged END
cannot close it); unit-name and argv validation; dedupe and rate limit;
settings parsing; `HeldDraft`; a source scan that the notification code never
reaches `send_prompt`/`Session::prompt`.

## 6. Verification (safe commands only)

`cargo build --all-targets`, `cargo test`, `cargo clippy --all-targets -- -D
warnings`, `cargo fmt --all --check` (devShell); `nix build`;
`checks.x86_64-linux.{policy,contract,core,panel,module,extension}`;
`scripts/preflight.sh` (on a tree copy if files are untracked). Never
`nix flake check`.

## 7. Sources

1. coredumpctl(1), freedesktop.org: matches, `info` fields, "not finding any
   matching core dumps is treated as failure".
2. systemd `src/coredump/coredump-submit.c`: file name
   `core.<comm>.<uid>.<boot-id>.<pid>.<usec>`, comm escaped for `./ `,
   `link_tmpfile`, mode 0640 + user read ACL.
3. systemd `src/shared/journal-util.c`: the access hint at `log_notice`
   (suppressed by `-q`), "No journal files were opened due to insufficient
   permissions."
4. `vte4` 0.10.0 source: `text_range_format` (`v0_72`), `cursor_position`,
   `column_count`.
5. `gio` 0.22.10 source: `Subprocess::newv`, `communicate_future`;
   `glib` 0.22.10: `future_with_timeout`.
6. systemd D-Bus API (`org.freedesktop.systemd1.Manager.ListUnitsFiltered`,
   `a(ssssssouso)`), and OWASP LLM01 prompt injection: delimiting untrusted
   data reduces but does not remove the risk.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Prompt injection through logs | Fence + fixed instruction; the real protection stays the approval cards and the managed policy. Stated in the results. |
| A secret the scanners miss | "Obvious" patterns only; the user sees the whole draft before Send |
| Notification spam | Baseline, per-key dedupe and cooldown, global cap |
| Any session-bus client can call the new actions | Targets re-validated; the effect is only a draft in the input |
| journalctl/coredumpctl not on PATH | Found via PATH like `just`; a failure gives a fixed reason in the draft |
