# CLAUDE.md
**vexos-ai** — Install-only Nix flake (NixOS module + packaged shell CLI + Rust assistant panel + GNOME Shell extension + agent skills + managed agent safety policy), consumed as a flake input by vexos-nix. Languages: Nix, Bash (shellchecked), Rust, GJS (extension), Markdown (skills and docs); Bats for tests. Frameworks: NixOS module system (`nixosModules.default`, option set `programs.vexos-ai`), nixpkgs `writeShellApplication` and `buildRustPackage`, GTK4/libadwaita, Claude Code and OpenCode managed settings. Package manager(s): Nix flakes (`flake.nix` + `flake.lock`, sole input `nixpkgs` at `github:NixOS/nixpkgs/nixos-26.05`) and Cargo (`Cargo.lock`, built through Nix).

## Commands

All `nix` commands need Nix with flakes on Linux or WSL.

Build:
- `nix build` (builds `packages.<system>.default`; `writeShellApplication` runs shellcheck at build time, so this is the lint gate)
- `nix eval .#nixosModules.default --apply 'm: builtins.typeOf m'` (confirms the NixOS module evaluates; mirrors CI)

Test (one check at a time; use `x86_64-linux` unless you have an aarch64 builder):
- `nix build .#checks.x86_64-linux.policy` (the safety policy in `nix/policy.nix`: never apply, always ask)
- `nix build .#checks.x86_64-linux.contract` (the bats suite in `tests/contract/` against the CLI under stubs; verifies `docs/contract.md`)
- `nix build .#checks.x86_64-linux.core` (ACP core and view-model tests, plus clippy)
- `nix build .#checks.x86_64-linux.panel` (the GTK panel builds and passes clippy)
- `nix build .#checks.x86_64-linux.module` (the NixOS module's assertions)
- `nix build .#checks.x86_64-linux.extension` (the GNOME Shell extension as built)
- `nix develop -c cargo fmt --all --check`

Preflight (final gate): `scripts/preflight.sh` (package build, the six checks above, module eval, rustfmt)
Fast verify (also run by the Stop hook): `scripts/preflight.sh`

## Repository

- Key directories:
  - `bin/` — `vexos-ai.sh`, the launcher and helpers; embedded into the package by `nix/package.nix`
  - `nix/` — `package.nix` (shellchecked CLI + desktop entry), `module.nix` (`programs.vexos-ai`), `policy.nix` (managed agent safety policy), `panel.nix`, `extension.nix`
  - `crates/` — Rust workspace: `vexos-ai-core` (ACP client), `vexos-ai-viewmodel` (panel rules, no GTK), `vexos-ai-panel` (GTK4 window)
  - `extension/` — the GNOME Shell extension
  - `skills/` — agent skills (`vexos`, `vexos-diagnose`) linked into `~/.claude/skills` and `~/.agents/skills`
  - `docs/` — `contract.md` (argv, exit codes and `status --json` for callers such as VexPortal) and `spec.md` (roadmap and decisions)
  - `tests/contract/` — Bats suite plus PATH stubs (`stubs/`)
  - `scripts/` — `preflight.sh`, `verify-host.sh` (read-only post-rebuild host check)
- Architecture: Install-only Nix flake: package + NixOS module + overlay + devShell, with the CLI machine contract (`docs/contract.md`) and the agent safety policy (`nix/policy.nix`) each enforced by a flake check
- CI environment: GitHub Actions (`ubuntu-latest`, `DeterminateSystems/nix-installer-action`), running `nix build`, `nix flake check` and a module `nix eval` on push to `main` and on pull requests
- OS requirements: Linux only — the flake exposes only `x86_64-linux` and `aarch64-linux`. `nix` commands cannot run on native Windows or macOS. The shipped CLI also needs a Linux desktop session at runtime (systemd user services, zenity, libnotify, xdg-terminal-exec), but tests stub these out
- Build layout constraints: `checks.<system>.<name>` is the way to run one check; `nix flake check` evaluates all of them for all systems and is forbidden
- Large disk side-effects: builds fill `/nix/store`; `nix build` also creates `./result` symlinks (gitignored)
- Other constraints: `nix flake update` or any `flake.lock` change must be a deliberate, user-visible change. vexos-nix pins the same `nixos-26.05` revision, so bumping the lock here must be followed by a lock bump in vexos-nix
- Special constraints: see Project-specific rules

### Project-specific rules

- The feature toggle (`vexos.features.ai.enable`), role gating and the `pkgs.unstable` package choice live in vexos-nix `modules/ai.nix`, not here
- `skills/` describe vexos-nix's justfile and `/etc/nixos` side-files. When those change in vexos-nix, update the skills here and bump the lock there
- `nix/policy.nix` is the assistant's safety policy, asserted by `checks.policy`: agents must never be able to apply a configuration (`nixos-rebuild switch|boot|test`, `just rebuild|switch|update|update-all`) or run `nix flake check`, and must always ask before acting (no Claude auto or bypass mode; OpenCode `ask` for edit and bash). Any change to `nix/policy.nix` must keep `checks.policy` in `flake.nix` in step with it
- Validate with `nix build` here and `dry-build` in vexos-nix; never `nixos-rebuild switch|boot` from automation
- `docs/contract.md` is a public contract for callers (VexPortal, the GNOME panel, tests); `tests/contract/*.bats` must keep passing unchanged against `bin/vexos-ai.sh` and its planned Rust replacement. A change to argv, exit codes or `status --json` is a contract change and needs `docs/contract.md` and the Bats suite updated together
- Which check covers which change: `nix build` for `bin/vexos-ai.sh` or `nix/package.nix`; the module `nix eval` for `nix/module.nix` or `flake.nix`; `checks.policy` for `nix/policy.nix` or the `policy` check; `checks.contract` for `bin/vexos-ai.sh`, `docs/contract.md` or `tests/contract/`; `checks.core`/`checks.panel` for `crates/`; `checks.extension` for `extension/`
- Context7 documents libraries, not nixpkgs or NixOS options; for nixpkgs packages and NixOS options use the `nixos` MCP server (`mcp__nixos__nix`) as the authoritative source

## Forbidden commands

Never run these. They are also blocked in `.claude/settings.json` where possible.

### Universal (all repositories)

- `git add` (any form)
- `git commit` (any form, including `--amend`)
- `git push` (any form, including `--force`)
- `git stash` (any form, including `pop`, `apply`, `drop`)
- Any other git command that stages, commits, pushes, or stashes changes

What gets staged, committed, pushed, or stashed is the user's decision; the commit message at the end of every change is what the user runs. Read-only git (`status`, `log`, `diff`, `show`) is allowed.

### Project-specific

- `nixos-rebuild switch`, `nixos-rebuild boot`, `nixos-rebuild test` (also with a `sudo ` or `pkexec ` prefix) — reason: applies a system configuration; destructive to the running system. This repo's `nix/policy.nix` denies these to the assistant it ships, and `checks.policy` asserts it. Use instead: `nix build` here, `dry-build` in vexos-nix
- `just rebuild`, `just switch`, `just update`, `just update-all` (also with a `sudo ` or `pkexec ` prefix) — reason: vexos-nix justfile recipes that apply or update the system; the user runs these. Use instead: none — ask the user
- `nix flake check` (also with a `sudo ` or `pkexec ` prefix) — reason: evaluates and builds every flake output for every system at once; the safety policy forbids agents from running it. CI runs it. Use instead: `scripts/preflight.sh` or `nix build .#checks.x86_64-linux.<name>`
- `nix-collect-garbage`, `nix store gc` — reason: deletes store paths shared with the rest of the machine. Use instead: none — ask the user

## How to work

### Task size

State the tier in one line before starting.

- **Tier 1 — ask or explore** (questions, reading code, anything that changes no files): just answer. No commit message.
- **Tier 2 — standard change** (bug fix, small feature, config or docs change, tests; a few files, no new dependency, no change to a public interface, schema, or build/CI config): research → brief plan in chat (goal, assumptions, docs consulted, steps with how each is verified) → implement → verify → preflight → commit message.
- **Tier 3 — major change** (new dependency; many files or modules; public interface, schema, or migration; security-sensitive code; build, CI, or deploy config; hard to reverse): in plan mode: research → full plan (including docs consulted) → **wait for approval** (the user approving the plan) → implement → verify → independent review → preflight → commit message.

If a task fits two tiers, use the larger. If a Tier 2 task grows, stop, say so, and continue as Tier 3. The user can ask for more or less process ("just do it", "full process"); verification, forbidden commands, and the commit message still apply.

Plan mode: for Tier 3, and before any new feature or design decision with real tradeoffs, enter plan mode yourself if you can; otherwise ask the user to switch (Shift+Tab) before planning. With the `opusplan` model setting, planning then runs on Opus and implementation on Sonnet.

### Principles

1. **Think before coding.** State your assumptions. If a request has several valid readings, present them or ask; never pick one silently. If a simpler approach exists, say so.
2. **Simplicity first.** Write the minimum code that solves the problem: no extra features, no abstractions for single-use code, no unrequested configurability, no handling for cases that cannot happen. If 200 lines could be 50, rewrite it.
3. **Surgical changes.** Every changed line must trace to the request. Do not refactor or tidy adjacent code; match the existing style. Remove only the imports, variables, and functions your change orphaned. Mention unrelated dead code instead of deleting it.
4. **Goal-driven.** Turn the task into a check you can run before you start: bug fix → a failing test first, then make it pass; new behaviour → tests for it; refactor → tests pass before and after. For multi-step work, list the steps with "→ verify: …" for each.

### Done means verified

- Do not call work finished until the build and tests have actually run and you have reported the real output. Never write "passes" for something you did not run.
- After a scripted or multi-file edit, confirm it applied (`git diff`) before running checks. Formatters can change lines between your edit and the check.
- Final gate: run the preflight command; the exit code must be 0. If none exists, run the build and test commands instead, say so, and offer to create a preflight script (do not create it unasked). A failure means the work is not finished.
- A Stop hook (`.claude/hooks/verify.sh`) re-runs the fast verify command when files have changed and blocks finishing on failure. Treat its report as a failed verification: fix it and re-run.
- Mechanical bugs in tooling or scripts that block a check (a shell error, a typo, a broken path) are fixed without asking, reported in one line, and listed in the commit message. Anything that changes behaviour still needs the user.
- If your first attempt at a bug fix fails, stop and re-plan before the next attempt (plan mode for Tier 3, a revised plan in chat for Tier 2). Do not stack more edits on an approach that did not work.
- Do not create spec, plan, or review files in the repository unless asked.

### Review findings

- Tier 3: before preflight, have the `reviewer` subagent review the diff. Give it the approved plan and the changed files, not your conclusions. If it is unavailable, review `git diff` yourself and say it was a self-review.
- Fix every in-scope finding, CRITICAL or RECOMMENDED, without asking. Then re-run the checks and the review. Repeat until the review has nothing left that needs a code change.
- Ask the user before fixing a finding only if the fix would change behaviour the user specified, add or upgrade a dependency, touch state outside the repository, or go beyond the task's scope.
- After 3 fix cycles that still fail (a failed review or preflight, fixed, failed again), STOP and report everything. Do not loop silently.
- Done means nothing in scope is left to fix. Do not end with "not implemented", "known issues", or "things I didn't touch" lists. Anything genuinely out of scope gets one line.

### Outside the repository

- Never change state outside the repository without asking: containers, volumes, database roles or passwords, images, installed packages, system or user configuration. Reading it is fine.
- If the harness or a permission rule denies a command, do not work around it with another command. Say what was denied and ask.

### Do not guess state

Before saying what is committed, pushed, or up to date, check: `git status`; `git log --oneline origin/$(git branch --show-current)..HEAD` (empty output means fully pushed); `stat <lockfile>`. Never tell the user to push, commit, or update without checking first.

### Research before changing

For Tier 2 and Tier 3, research comes before the plan, and the plan shows what you found.

1. **Read the code first.** Read the files you will change and the nearest existing code that does something similar. Reuse its patterns and helpers.
2. **Check current docs for every external library, framework, CLI, or API the change uses or touches, including ones already in the lock file.** Look up the version in use (the manifest or lock file says which). Use the Context7 MCP if available (resolve the library, then query its docs); otherwise the official documentation. Do not rely on memory for signatures, options, flags, or default behaviour. When a failure involves a library, search its docs and issue tracker for the error before choosing a fix. You may hand lookups to the `docs` subagent (if present); it returns quoted excerpts with source and version. Check the excerpt supports its answer before relying on it.
3. **Tier 3 only:** for any dependency being added or upgraded, also read its release notes or changelog and check for deprecations and known advisories. When choosing between libraries, compare at least two.
4. **Show it.** The plan has a line `Docs consulted:` naming each source and what it settled. Write `Docs consulted: none needed — <reason>` only when the change uses no external library or API (pure internal logic, comments, docs, formatting). If a lookup fails or the docs contradict the code, say so and ask; do not guess.

### Communication

- Lead with the answer or result. Use short sentences and plain words.
- Include only what the user needs: the result, what changed, what they must decide, and real check output. Do not restate the request, narrate your steps, or summarize what you just showed.
- Prefer plain wording to jargon and abbreviations. If a technical term is needed, explain it once.
- Use a list only for three or more parallel items. No headings in short replies.
- This does not shorten required content: the plan, the `Docs consulted:` line, failure output, and the commit message stay complete.

### Commit message (final step of every change)

Every change that modifies files ends with a commit message, however small (a one-line typo fix included). It is the last thing in your reply. You never run `git commit`; the user does.

The user commits all changed files at once, so give exactly **one** commit message covering every uncommitted change in the working tree, including work from earlier tasks that has not been committed yet. Never split the work into several messages.

Format, ready to paste into `git commit`:

```
<type>(<scope>): <description — MAX 72 characters total>

<PARAGRAPH EXPLAINING WHAT CHANGED AND WHY>

Modified Files:
- path/to/file1
- path/to/file2
- path/to/file3

✔ Build successful
✔ Tests passed
✔ Review approved
✔ Preflight passed
```

Valid types: `feat`, `fix`, `chore`, `refactor`, `docs`, `test`, `perf`. Example first line: `fix(network): disable swap on ZFS server roles`

Inside the message: no "Commit Message" heading, no "Edited" summary, no diff statistics, nothing outside the template. List new files under Modified Files too.

Checkmark lines appear only for checks that actually ran and passed on the final code in this session; never include a line for a skipped check. `✔ Review approved` means the Tier 3 review returned PASS (write `✔ Review approved (self-review)` if no independent reviewer was used). Omit the Preflight line if no preflight exists.

After the message, if every check that ran passed, add one line:
> All checks passed. Code is ready to push to GitHub.
