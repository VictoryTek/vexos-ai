# Platform boundary — spec

Status: **notes only**, 2026-10-27. No source changed. The only code-adjacent
change is a non-blocking Windows CI job,
`.github/workflows/windows-portable.yml`, which has **not run yet** (it runs on
the first push or pull request).

Goal: keep vexos-ai one project that can later ship both as a Nix flake output
and as a Windows installer. This note records where the code assumes Linux or
NixOS today, which crates are already portable, and where a platform boundary
should go. It decides nothing about Windows UI, policy or packaging; those are
listed as open decisions in section 7.

## 1. Inventory: Linux and NixOS assumptions

| Area | Where | Assumption |
| --- | --- | --- |
| Apply changes (rebuild) | `crates/vexos-ai-viewmodel/src/rebuild.rs:7-8` | Fixed argv `just rebuild`, fixed dir `/etc/nixos` |
| | `crates/vexos-ai-viewmodel/src/rebuild.rs:84-94` | `describe_wait_status` decodes a Unix `waitpid` status (signal in the low 7 bits). Compiles anywhere; meaningless for a Windows exit code |
| | `crates/vexos-ai-panel/src/rebuild.rs` | Runs the argv in a VTE terminal (`vte4`, `spawn_async`, `PtyFlags`) |
| | `bin/vexos-ai.sh:29` | `WORKDIR=/etc/nixos` |
| Agent working directory | `crates/vexos-ai-panel/src/main.rs:25,333` | `WORKDIR = "/etc/nixos"` passed to `Config::new` |
| Diagnostics collectors | `crates/vexos-ai-viewmodel/src/diagnose.rs:194-215` | Fixed argv for `journalctl` and `coredumpctl info` |
| | `crates/vexos-ai-viewmodel/src/diagnose.rs:226-260` | Parses systemd-coredump file names (`core.<comm>.<uid>.<boot id>.<pid>.<usec>`); `uid` is a Unix uid |
| | `crates/vexos-ai-viewmodel/src/diagnose.rs:301-330, 363` | Interprets journalctl/coredumpctl output and the `systemd-journal` group hint |
| | `crates/vexos-ai-viewmodel/src/diagnose.rs:341` | Draft text names `just rebuild` in `/etc/nixos` |
| | `crates/vexos-ai-viewmodel/src/redact.rs:399` | Home-directory redaction looks for `/home/` only (Windows: `C:\Users\…` not redacted) |
| | `bin/vexos-ai.sh:841-859` | `nixos-rebuild list-generations`/`dry-build`, `systemctl --failed`, `journalctl` |
| Notifications and watchers | `crates/vexos-ai-panel/src/notify.rs:12,46-47` | `std::os::unix::fs::MetadataExt`, uid from `/proc/self` |
| | `crates/vexos-ai-panel/src/notify.rs:27` | `/var/lib/systemd/coredump` |
| | `crates/vexos-ai-panel/src/notify.rs:201-210` | systemd1 over the system D-Bus (`ListUnits` for failed units) |
| | `crates/vexos-ai-panel/src/notify.rs:136-163` | `gio::Notification` through the GApplication |
| | `crates/vexos-ai-viewmodel/src/notify.rs:9-16` | Settings file under `$XDG_CONFIG_HOME` (relative path itself is portable) |
| | `bin/vexos-ai.sh:68,943-981` | `notify-send`, `journalctl -f` crash watcher |
| | `nix/module.nix:157-178` | systemd user services and timer (crash-watch, theme-watch, welcome, usage) |
| Policy install and paths | `nix/policy.nix` (content), `nix/module.nix:150-151` | `/etc/claude-code/managed-settings.d/50-vexos.json`, `/etc/opencode/opencode.json` |
| | `nix/module.nix:189-198` | Panel refuses to install without those files (module assertions) |
| | `flake.nix` `checks.policy` | Asserts the rendered policy; the paths themselves are asserted by `checks.module` |
| Skills | `nix/module.nix:138-146` | `/etc/vexos/ai/skills`, linked by `systemd.user.tmpfiles` into `~/.claude/skills` and `~/.agents/skills` |
| | `skills/` | Describe vexos-nix's justfile and `/etc/nixos` |
| Extension, D-Bus activation | `extension/`, `nix/extension.nix` | GNOME Shell 50 extension |
| | `nix/panel.nix:36-45`, `nix/module.nix:210` | D-Bus service file, `services.dbus.packages`; notification clicks need bus name = desktop id (`checks.module`) |
| | `crates/vexos-ai-panel/src/main.rs:53` | `adw::Application` with `APP_ID` (GApplication/D-Bus) |
| UI toolkit | `crates/vexos-ai-panel/Cargo.toml` | GTK4 0.11, libadwaita 0.9, vte4 0.10 |
| | `nix/panel.nix:49` | `platforms = lib.platforms.linux` |
| CLI | `bin/vexos-ai.sh` | Bash, `xdg-terminal-exec`, `zenity`, `systemctl --user` |
| Spike harness | `spike/acp/sandbox.sh` | `bwrap` mount namespace, binds policy files at `/etc/...`, shadows `/run/current-system/sw/bin/nixos-rebuild` |
| Tooling | `scripts/preflight.sh`, `scripts/verify-host.sh`, `flake.nix` | Nix on Linux/WSL only; flake systems are `x86_64-linux` and `aarch64-linux` |

## 2. Portability today

| Crate | Portable? | Why |
| --- | --- | --- |
| `vexos-ai-core` | **Yes, to compile** (predicted) | Depends only on `serde_json` and `tokio` (`process`, `io-util`, `rt`, `sync`, `time`), all of which support Windows. No `cfg(unix)`, no `std::os::unix`, no shell. Agent launch has Windows gaps (below) |
| `vexos-ai-viewmodel` | **Yes, to compile** (predicted) | `pulldown-cmark` (no default features), `serde_json`, the core. Pure logic. The rebuild and diagnose modules compile but describe Linux-only behaviour |
| `vexos-ai-panel` | **No** | GTK4, libadwaita, VTE, `std::os::unix`, `/proc`, systemd D-Bus. Excluded from the Windows job |

### Predicted Windows CI results (by reading; nothing was run on Windows)

The job builds and tests only `-p vexos-ai-core -p vexos-ai-viewmodel` with
`--locked`, then clippy `-D warnings` and `cargo fmt --check`.

1. **Build: expected to pass.** No platform-specific API in either crate.
2. **Tests: `vexos-ai-core/tests/session.rs` expected to fail almost entirely.**
   `config()` at `crates/vexos-ai-core/tests/session.rs:89` uses
   `Config::new("/tmp/vexos-ai-test")`. `Session::start` rejects a
   non-absolute cwd (`crates/vexos-ai-core/src/session.rs:210-211`), and on
   Windows `Path::is_absolute` needs a drive or UNC prefix, so `/tmp/...` is
   relative. Every test that starts a session through `config()`, `claude()`
   or `opencode()` should fail at `Session::connect` with
   `Error::Config("the working directory must be absolute")`. The one test that
   passes `Config::new("relative")` (`tests/session.rs:908`) expects that error
   and should still pass. The fake agent itself is an in-memory
   `tokio::io::duplex`, not a script, so it is portable.
3. **Viewmodel tests: expected to pass.** Paths such as `/etc/nixos/a.nix`,
   `/tmp/plan.txt`, `/home/…` and `file:///etc/shadow` in
   `proposal.rs:171-239`, `markdown.rs:363-388`, `redact.rs:578-599`,
   `diagnose.rs:565,638` and `rebuild.rs:112` are plain strings compared as
   strings, never touched on disk. `rebuild.rs:144` tests Unix wait-status
   arithmetic, which is pure integer math.
4. **Clippy and rustfmt: expected to pass.** No `cfg` branches, so the same
   code is linted. `.gitattributes` forces `eol=lf`, so the checkout has LF and
   rustfmt sees the same files as on Linux.
5. **Not caught by CI, because no test spawns a real agent:**
   - `Session::spawn` runs `Command::new("claude-agent-acp")` and
     `Command::new("opencode")` (`crates/vexos-ai-core/src/agent.rs:38-52`,
     `session.rs:165`). On Windows, Rust's `Command` appends `.exe` when the
     name has no extension but finds no other extension (Rust `std::process::Command`
     docs). npm installs a `.cmd` shim for a package bin, so an npm-installed
     agent would not be found by bare name. Spawning the `.cmd` instead runs it
     through `cmd.exe`, whose argument decoding the Rust docs warn about. The
     argv is fixed here (`[]`, `["acp"]`), which limits that risk.
   - `kill_on_drop`/`start_kill` (`session.rs:172,296,603`) call
     `TerminateProcess` on the direct child only. If the child is a `.cmd`
     shim (`cmd.exe` → `node`), the agent process may outlive the session.
     **Not verified**; needs a test on Windows.
   - `stderr(Stdio::inherit())` (`session.rs:171`) has no console to inherit
     from in a Windows GUI-subsystem app. Harmless, but the agent's stderr is
     lost.
   - The ACP `cwd` is sent as the platform path string (`session.rs:351-357`);
     the agents' handling of Windows paths over ACP is untested.

## 3. Proposed boundary (not implemented)

Rule: `vexos-ai-core` stays platform-free and owns every safety rule. Platform
facts reach it only as data (a resolved program path, a cwd). Anything that
runs a platform command, reads platform logs or posts a notification sits
behind a trait in a new crate, with one implementation per platform. The
view-model depends on the traits and on the core, never on an implementation.

Proposed crate: **`vexos-ai-platform`** (traits and shared types only, no
dependencies beyond the core types). Implementations in
**`vexos-ai-platform-linux`** (moved from today's panel and view-model) and,
later, **`vexos-ai-platform-windows`**. The binary crate (GTK panel today,
whatever the Windows UI becomes) picks one at startup. Owners: the core's
maintainer owns the trait crate and its tests; each platform crate is owned
with its UI shell.

| Interface | Trait (in `vexos-ai-platform`) | Linux impl (today's code) | Windows impl (to design) |
| --- | --- | --- | --- |
| Apply changes | `ApplyAction`: `fn command(&self) -> FixedCommand` (argv, dir; no input), `fn describe_exit(&self, raw: i64) -> String` | `["just","rebuild"]` in `/etc/nixos`; `describe_wait_status` | Undecided (depends on 7.1). The confirm-once state machine (`Rebuild`, `RebuildCommand`) stays in the view-model |
| Diagnostics collectors | `Collector`: `fn argv(&self, target: &Target) -> Vec<String>`, `fn interpret(&self, success, stdout, stderr) -> Log` | `journal_argv`, `coredump_argv`, `journal_log`, `coredump_log`, `core_file` | e.g. Event Log / WER queries; to be researched |
| Watchers | `Watcher`: yields `Crash`/`UnitName`-like events into `notify::Notifier` | coredump dir, systemd1 D-Bus | to be researched |
| Notifier | `Notifier`: `fn post(&self, id, title, body, action)` | `gio::Notification` | Windows toast (toolkit-dependent) |
| Policy locations | `PolicyPaths`: where Claude's and OpenCode's managed files must exist, and a check that they match the rendered policy | `/etc/claude-code/managed-settings.d/50-vexos.json`, `/etc/opencode/opencode.json` | `C:\Program Files\ClaudeCode\` (file or `managed-settings.d/`) or `HKLM\SOFTWARE\Policies\ClaudeCode`; `%ProgramData%\opencode` |
| Agent launcher and path resolution | `AgentLocator`: `fn resolve(&self, agent: Agent) -> Result<PathBuf>` | PATH lookup (the module puts pinned agents first on PATH) | Resolve `.exe` (or a vetted shim) to an absolute path, then hand it to the core |
| Config and home paths | `Dirs`: config dir, home prefix for redaction | `$XDG_CONFIG_HOME`, `/home/` | `%APPDATA%`, `C:\Users\` |

Changes this implies, for later: `AgentCommand.program` becomes a resolved
path supplied by the caller (the core still fixes args and env); `redact.rs`
takes home prefixes from `Dirs`; `diagnose.rs` and `rebuild.rs` move their
Linux constants into `vexos-ai-platform-linux`. The view-model's draft and
redaction logic stays where it is.

## 4. Safety constraints that must survive any platform

These live in **`vexos-ai-core`** and must not be reimplemented per platform.
A platform crate may only add restrictions.

| Constraint | Where today |
| --- | --- |
| Modes pinned: Claude `default`/`plan`, OpenCode `build`; any other or unreadable report resets; a failed reset aborts | `crates/vexos-ai-core/src/agent.rs:22-35`, `session.rs` |
| No API to choose a mode or agent | `crates/vexos-ai-core/src/lib.rs:5-10` |
| Approval options filtered to approve once / deny once | `crates/vexos-ai-core/src/permission.rs` |
| Deny by default: timeout, cancel, drop, closed stream all deny | `crates/vexos-ai-core/src/session.rs` |
| No auto-approve mode, flag or environment variable set by the client | `crates/vexos-ai-core/src/agent.rs:38-52` (fixed env) |
| No fs or terminal capability offered to the agent | `crates/vexos-ai-core/src/session.rs:333` |

Owned by the **view-model** (shared, also not per platform): agent output is
untrusted (`diagnose.rs:5`, `redact.rs`, `markdown.rs` with no HTML), the
apply action runs only after a click and a confirmation and is never
reachable from a conversation (`rebuild.rs:1-3`).

Owned by **each platform implementation**, and so needing its own tests:
collectors read-only with a fixed argv and no shell; the apply action spawned
without a shell; the managed policy installed where the agent reads it and not
writable by the user. Note that on Windows the HKCU policy key is
user-writable and Claude Code ranks it below any admin document; on WSL
`/etc/claude-code` is user-writable too (Claude Code managed settings docs).
Neither may be the only policy source.

## 5. Re-test on Windows before any release

1. **Escalation routes from the spike**
   (`assistant_panel_spike_results.md`):
   - settings-file modes: user-level `defaultMode: acceptEdits` starts a
     session in `acceptEdits` (test 4); project `.claude/settings.json` and
     `settings.local.json` modes;
   - subagent permission modes: project and user `.claude/agents/*.md` with
     `permissionMode: acceptEdits` (P4, closed on Linux by managed `ask`
     rules);
   - mid-session mode switches (R5, `set_mode acceptEdits`);
   - OpenCode custom agents (R2, still open on Linux).
2. **The approval filter**: only approve once / deny once reach the UI, against
   the real Windows agents.
3. **Policy file locations**: that each agent reads the Windows managed
   location, that a missing or unreadable file fails closed, and the precedence
   between HKLM, `C:\Program Files\ClaudeCode` and HKCU.
4. **Agent launch**: path resolution, `.cmd` shims, and that dropping a
   session ends the whole agent process tree (section 2, item 5).
5. **Deny list**: the Windows equivalent of "never apply" (depends on 7.1).

The Linux harness does not carry over: `spike/acp/sandbox.sh` needs `bwrap`
mount namespaces to place policy files at `/etc/...` without installing them,
and shadows `nixos-rebuild` through `/run/current-system`. Windows has no
equivalent here; tests would need a disposable VM or Windows Sandbox with
admin rights to write `C:\Program Files\ClaudeCode` or HKLM. **Not
researched.**

## 6. CI

`.github/workflows/windows-portable.yml`: `windows-latest`, Rust 1.95.0 (the
`rustc` in nixpkgs `nixos-26.05`, confirmed with the nixos MCP; also the
crates' `rust-version`), `actions/checkout@v7` (latest release v7.0.1, checked
on its releases page), `dtolnay/rust-toolchain@1.95.0` (README: the toolchain
follows the ref; the `1.95.0` ref exists). Job-level `continue-on-error: true`
("Prevents a workflow run from failing when a job fails", GitHub workflow
syntax docs), and a separate workflow so `ci.yml` is unchanged. Whether the
failed job still shows as a red check on a pull request was **not verified**;
it is not required by branch protection, so it cannot block a merge.

To make it required: fix the section 2 failures, remove `continue-on-error`,
add the check to the branch protection rule for `main`.

Seen while checking versions, not changed (out of scope):
`ci.yml` pins `actions/checkout@v4` and
`DeterminateSystems/nix-installer-action@v13`; the latest releases are
v7.0.1 and v23. `ci.yml` also runs `nix flake check`, which agents may not
run locally; that is CI's choice.

## 7. Open decisions (for the user)

### 7.1 Native Windows app or an installer that sets up WSL

| | Native Windows | Installer that sets up WSL |
| --- | --- | --- |
| What gets reused | Core, view-model; new UI shell and platform crate | Almost everything: NixOS-WSL could run the same module, CLI and skills |
| What "apply changes" means | Undecided: there is no `/etc/nixos`; the product's purpose on Windows must be defined first | The same `just rebuild` inside the distro |
| Policy | Windows managed locations (section 4) | `/etc/claude-code` inside WSL is user-writable per Claude Code docs, unless `wslInheritsWindowsSettings` makes WSL read the Windows policy chain; so a WSL install still needs a Windows-side admin policy |
| UI | New toolkit (7.2) | GTK under WSLg, or a Windows UI talking to the WSL side |
| Unverified | Agent behaviour on Windows over ACP | NixOS-WSL fit for this module (D-Bus, systemd user services, GNOME extension do not apply); WSLg notification and tray integration |

### 7.2 Windows UI toolkit

Candidates: GTK4 on Windows (keeps the panel, but VTE has no Windows port, so
the rebuild terminal must change), a native Rust toolkit, or a web-view shell.
**Not researched**: maturity, accessibility, toast support and packaging of
each. Whatever is chosen only implements section 3's traits and renders the
view-model.

### 7.3 Building and signing the .msi/.exe

Options to compare: WiX Toolset, `cargo-wix`, `cargo-dist`, MSIX. Signing:
an Authenticode certificate (OV/EV) or a cloud signing service. CI would need
a Windows runner and the signing secret. **Not researched**: current tool
versions, certificate costs, SmartScreen reputation behaviour. Nix cannot
build Windows installers in this flake (systems are Linux only), so this is a
separate build path.
