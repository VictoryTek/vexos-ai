# Shell panel spike (Phase 0)

Throwaway. A static drop-down chat panel drawn inside GNOME Shell 50 with St and
Clutter, in the style of Looking Glass, with **fake data only**. It answers the
Phase 0 questions in
[shell_native_panel_spec.md](../../.github/docs/subagent_docs/shell_native_panel_spec.md)
(results are at the end of that note).

It is a separate extension, `vexos-shell-spike@victorytek.github.io`. It is not
packaged, not installed by any module, and nothing in the repo enables it. (`harness/start.sh` enables it, but only inside its own nested shell.)

What it has no part in: no helper, no D-Bus call, no agent, no file read, no
network, no timer. Closed, it holds only its keybinding (`Super+F10`).

## Keys

| Key | Does |
|---|---|
| `Super+F10` | opens or closes the panel (also works while it is open) |
| `Escape` | closes it |
| `Enter` | "sends": adds the text as a row (nothing leaves the Shell) |
| `Shift+Enter` | new line in the input |

The panel also has a Close button, a Copy button per row, a fake approval card
(two buttons that only change its status line) and **Throw a test error**, which
throws on purpose to show the panel closes and the Shell survives.

## Tests (no Shell needed)

```
node --test spike/shell-panel/tests/logic.test.mjs
```

Pure logic (`logic.js`: text escaping, size caps, geometry) and source scans of
`extension.js` (no markup, no timers, no I/O, one stage signal, every handler
guarded, nothing else in the repo names the spike). `SPIKE=<dir>` runs them
against a copy of the extension directory. `tests/mutate.py make DIR` then
`tests/mutate.py run DIR` is the mutation check (30 one-line mutations; every
one must make a test fail).

## Nested shell (this repo's machine, not your session)

`harness/start.sh` starts a headless GNOME Shell 1280x800 on a private session
bus with every directory under `target/shell-spike-run/`, enables the spike and
a test-only helper that allows `Eval`/`Screenshot`, and returns when it answers.
`harness/stop.sh` stops only processes whose environment names that run
directory.

```
spike/shell-panel/harness/start.sh
spike/shell-panel/harness/keys.sh super+F10            # virtual keyboard
spike/shell-panel/harness/keys.sh text:hello shift+Return text:world
spike/shell-panel/harness/shot.sh name                 # target/shell-spike-run/shots/name.png
spike/shell-panel/harness/ev.sh 'Main.overview.visible'
spike/shell-panel/harness/stop.sh
```

## Manual test in the VM

A Shell bug can end the whole graphical session. Save your work first. Run these
in the VM, not on a machine you are working on.

**Install** (nothing here is done by the repo):

1. Copy the extension; the directory name must equal the uuid:
   `mkdir -p ~/.local/share/gnome-shell/extensions && cp -r spike/shell-panel/vexos-shell-spike@victorytek.github.io ~/.local/share/gnome-shell/extensions/`
2. Compile its settings schema:
   `nix shell nixpkgs#glib.dev -c glib-compile-schemas ~/.local/share/gnome-shell/extensions/vexos-shell-spike@victorytek.github.io/schemas`
3. Log out and log in (Wayland finds new extensions only at login).
4. `gnome-extensions enable vexos-shell-spike@victorytek.github.io`, then
   `gnome-extensions info vexos-shell-spike@victorytek.github.io` should say
   `State: ACTIVE`. If it says ERROR, read
   `journalctl --user -b -g vexos-shell-spike` or `journalctl -b -g gnome-shell`.

**Try** (write down pass or fail; the nested-shell result is in the note):

1. Open and close: `Super+F10` opens, again closes; open, `Escape` closes.
   Press `Super+F10` ten times quickly: it must never get stuck half open.
2. Slide: does it come down from under the top bar and go back smoothly, with no
   jump in the first frames? Try a 60 Hz and a high refresh monitor if you have
   both. Try it with the overview open, and with a fullscreen window behind.
3. Input: type; `Shift+Enter` three times (box grows to at most 8 lines, then
   scrolls); delete (box shrinks); paste a long text from another app
   (`Ctrl+V`); `Ctrl+A`, `Shift+Home`, mouse selection; `Ctrl+C`/`Ctrl+X`.
   `Enter` adds a row and the chat scrolls to the bottom. Try your own keyboard
   layout and, if you use one, an input method (IME): compose a character, and
   press Enter to commit the composition: it must **not** send the message (a
   known risk: Enter is handled before the input method sees it). `Ctrl+Z` is expected to do nothing.
4. Replies: drag over a reply to select it; double-click a word; press Copy and
   paste elsewhere. Note: selected text may be unreadable (white on light).
5. Grab: with the panel open, type: the keys go to the panel, not the window
   behind. Close with `Super+F10`; keys go to the window again and focus is
   where it was. Open a terminal and an app first and check which one has
   focus after closing.
6. Not a window: with the panel open press `Alt+Tab` and open the overview: there
   must be no entry or dock icon for it.
7. Errors: press **Throw a test error**: the panel must close, the desktop must
   keep running, and `journalctl -b -g vexos-shell-spike` must show one line,
   `handler error: SpikeTestError`, with no message text. Then open it again.
8. Screen lock (`Super+L`) with the panel open: after unlocking the panel must be
   gone and `Super+F10` must work. Disable the extension while the panel is open
   (`gnome-extensions disable ...`): it must disappear at once and leave nothing.
9. The long and odd messages (very long text, long code block, wide line, markup
   characters, control and bidi characters) must show as plain text, with
   `\u{...}` for control and direction characters, and a "shortened" mark on the
   two long ones.
10. If you have a screen reader (Orca), HiDPI, or a second monitor, note what
    happens; those were not tested. In particular put a monitor **above** the
    primary one: the panel hides above the primary monitor's top edge, so it may
    show on that monitor while it slides.

**Remove** (afterwards):

1. `gnome-extensions disable vexos-shell-spike@victorytek.github.io`
2. `rm -r ~/.local/share/gnome-shell/extensions/vexos-shell-spike@victorytek.github.io`
3. `dconf reset -f /org/gnome/shell/extensions/vexos-shell-spike/`
4. Log out and in; `gnome-extensions list` must not show it.
