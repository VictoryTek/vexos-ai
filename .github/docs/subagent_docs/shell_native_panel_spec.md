# Shell-native assistant panel: design note

Status: **design only**, 2026-10-30. Nothing is built. No code, Nix file, test or
other document was changed. **No build or check was run** (docs only, by the
request). Read first: `assistant_panel_spec.md`, `assistant_panel_app_spec.md`,
and in `assistant_panel_spike_results.md` the sections "Extension", "Docked
window", "Automatic usage", "Draft flow hardening", "Agent from settings" and
"Refinements"; `extension.js`, `layout.js`, the view-model and core public APIs.

Marks used below: **[src]** read this session (named); **[repo]** read in this
repo; **[unconfirmed]** I could not verify it, so it is a prediction, not a fact.

## 0. Answer first

**Recommendation: do a gated spike, not the rewrite.** Build only Phase 0 (a
static, fake-data panel drawn inside the Shell, section 9). Decide on the rest
after it answers four questions: does a growing multi-line input work, can the
user select and copy text, does the slide look smooth, and does the keyboard
grab keep the shortcut working. If the multi-line input fails, **do not do
this**: a chat panel without a usable input is not worth the extra trust placed
in the Shell.

Why it is worth a spike: GNOME ships a drop-down of exactly this shape inside
the Shell, Looking Glass, built from an `St.Entry`, `St.ScrollView`, a modal
grab and a `y` ease **[src: gnome-50 `js/ui/lookingGlass.js`]**. A panel drawn
there has no window, no map effect, no Wayland resize round trip and no process
to start before it can appear. Those are the causes of the slide and launch
problems in "Refinements".

Why only a spike: no extension I could find draws a chat UI in St (ddterm, the
precedent in `assistant_panel_spec.md` source 9, is a GTK app plus an extension;
its README says nothing about how it draws **[src: ddterm README, fetched]**).
Every public Shell API used is internal and unstable between GNOME releases.
And, the part to weigh most: **approvals become a message on the session bus,
which is weaker than a click inside one process** (section 3A, verdict).

**Verdict on approvals, in one line: yes, this design is weaker than the GTK
panel for approvals. The loss is moderate and limited to same-user attackers
and to display bugs (details in 3A.4), and part of it can be bought back.**

## 1. How shell-native panels are built

| Piece | What the GNOME Shell 50 source says | Use for us |
|---|---|---|
| Quick Settings | `QuickSettingsMenu extends PopupMenu.PopupMenu`; the menu is a `BoxPointer` in a wrapper actor; no `pushModal` in that file; focus group via `global.focus_manager.add_group`; **no `St.ScrollView`** **[src: gnome-50 `quickSettings.js`]** | A popup anchored to a button. Wrong shape for a tall chat. Not copied |
| BoxPointer animation | `POPUP_ANIMATION_TIME` 150 ms, `EASE_OUT_QUAD`; opacity, translation and scale set with `actor.ease(...)`; **no clip animation, no compositor-side effect** **[src: gnome-50 `boxpointer.js`]** | The Shell's own feel is 150 ms. Our docked slide is 250 ms [repo `layout.js`] |
| Looking Glass | Added with `Main.uiGroup.add_child` and placed **below `panelBox`** (`set_child_below_sibling`), so it slides out from under the top bar. `open()`: `Main.pushModal(this, {actionMode: Shell.ActionMode.LOOKING_GLASS})`, `show()`, ease **`y`** over 500 ms (divided by `St.Settings.slow_down_factor`) `EASE_OUT_QUAD`, `entry.grab_key_focus()`. `close()`: ease `y` back, then `popModal` and `hide()` in `onComplete`. Escape closes in `vfunc_key_press_event` **[src: gnome-50 `lookingGlass.js`]** | **The pattern to copy.** Also shows the top bar drawn over the sliding panel, as "Refinements" wanted |
| `pushModal` side effect | `global.stage.grab(actor)` sends all input to the panel while it is open **[src]**, so the user cannot click or type in another window (for example copy from a terminal into the chat), which the docked window allows today. Whether keyboard focus works without a grab is [unconfirmed]; both are Phase 0 questions | |
| `pushModal` | `pushModal(actor, params)`: `global.stage.grab(actor)`; the only option is `actionMode` (default `NONE`, "will filter out all keybindings"); saves and restores key focus; `popModal(grab)` dismisses it **[src: gnome-50 `main.js`]** | Needed for typing. **Risk:** with `NONE` our own shortcut stops working while open. Looking Glass defines its own action mode. We must pick one that keeps `toggle-assistant` alive, or handle the key in the panel. [unconfirmed which mode works; a Phase 0 question] |
| Chrome | `addChrome` adds to `uiGroup` below `top_window_group`; `addTopChrome` adds above all windows including popups; `trackFullscreen` hides it in fullscreen; `affectsStruts` makes it push windows **[src: gnome-50 `layout.js`]** | Looking Glass does not use either. For us, `affectsStruts` must stay false: a dropped panel must not shove windows around. Which of the three placements is best is a Phase 0 question |
| Scrolling | `St.ScrollView` is used in the date menu (`vscrollbar_policy: EXTERNAL`, `overlay_scrollbars`) and in Looking Glass (`scrollToBottom` by adjustment signals) **[src: `dateMenu.js`, `lookingGlass.js`]** | Chat scrolling is a known pattern |
| Message tray, overview | Not read this session (`messageTray.js`, `overview.js`). [unconfirmed] | Not needed for the design; do not rely on them |
| ddterm | Not drawn in St as far as I could tell; the README does not say **[src: ddterm README]**. `assistant_panel_spec.md` calls it a GTK app plus an extension [repo] | No precedent for a St chat UI |

**What gives an instant open and a smooth slide, and why.**

- *Instant:* the actors already exist in the Shell process. Open is
  `show()` + `ease`, in the same turn as the key press. Nothing waits for a
  process, a Wayland surface, a first frame from a client or a resize
  acknowledgement. Today the window path needs all four [repo: "Refinements"
  section 1].
- *Smooth:* the ease runs in the Shell's own frame clock on the actor tree, so
  it is not racing a client commit. The fragile steps in `extension.js`
  (`_park`, the promise-job ordering behind Shell's `_mapWindow`, the 400 ms
  fallback, `position-changed` waiting) all go away because there is no window
  to map.
- *What it does not make instant:* the agent. The session still has to start
  (`claude-agent-acp` handshake). That time was **never measured with a real
  agent** [repo: "Agent from settings" 5]. The panel can open at once showing
  "Starting…", as the GTK one does.

## 2. What St can and cannot do for our content

Ratings: **feasible**, **awkward**, **not feasible**. "Phase 0" = must be shown
working in the spike before anything else is built. Everything about `St` and
`Clutter.Text` properties below is **[unconfirmed]** for Shell 50: the GJS docs
page I fetched returned no content, so these come from memory of the API.

| Content (today's GTK) | Rating | Notes and workaround |
|---|---|---|
| Scrolling chat | feasible | `St.ScrollView` + `St.BoxLayout`, scroll to the bottom through the vertical adjustment, as Looking Glass does **[src]**. Cap the rendered rows (section 4) |
| Multi-line input that grows | **awkward, Phase 0 gate** | `St.Entry` is single-line by default; its `clutter_text` can be put in multi-line mode with wrapping (`single_line_mode = false`, `line_wrap = true`) [unconfirmed in 50]. Looking Glass only uses it single-line **[src]**. Enter vs Shift+Enter has to be handled in `key-press-event`. Growing height: natural height grows with lines, inside a `ScrollView` with a max height. IME, undo and spell check: IME via `input_purpose`/input method is plausible [unconfirmed]; undo/redo and spell check are probably missing. If this fails, stop |
| Text selection and copy of replies | awkward | `Clutter.Text` has a `selectable` property [unconfirmed]; a selectable, reactive label may let the mouse select, but Ctrl+C and a context menu are not guaranteed (Looking Glass uses `ShellEntry.addContextMenu` for its entry, which is what a copy menu would need **[src]**). Workaround that is certainly feasible: a **Copy** button per reply using `St.Clipboard.get_default().set_text(...)` [unconfirmed signature] |
| Read-only usage line | feasible | One `St.Label`. Its text is the view-model's `UsageButton` label, unchanged |
| Code blocks | feasible, **better than GTK** | One `St.Label` with a monospace class inside a styled `St.BoxLayout` gives a solid tinted block, which the GTK text-tag approach could not ("Panel polish 2": ragged per-line tint) [repo] |
| Aligned tables | feasible | Rust already pads columns into monospace text (`markdown.rs`) [repo]; render as a monospace label. Wide-character misalignment stays as today |
| Mixed styling inside a paragraph (bold, italic, inline code in one wrapped line) | **awkward** | Needs block structure from Rust first (section 6). St has no inline flow layout. Options: (1) a `Pango.AttrList` built from validated numeric ranges and set on the `Clutter.Text` [unconfirmed that GJS exposes the attribute constructors]; (2) style per block only and drop inline emphasis. **Not** Pango markup built from agent text (section 4) |
| Headings, lists | feasible | Block-level labels with a style class; list markers are literal text Rust already emits |
| Approval cards with buttons | feasible | `St.BoxLayout` + `St.Button` (the Shell is full of them). Keyboard focus must work under the modal grab [unconfirmed; Phase 0]. Accessible names: set `accessible_name` [unconfirmed] |
| Diff in a card | feasible | `DiffLine` list from Rust → one label per hunk or per line, green/red style classes. Colours would come from the Shell theme, not libadwaita, so the contrast work in "Panel polish 2" has to be redone and re-measured |
| Draft banner, replace/append/keep question | feasible | Inline row with three buttons instead of `adw::AlertDialog`. Same rules (they live in `Composer`) |
| Light/dark/high contrast | awkward | The Shell stylesheet decides. Our extension `stylesheet.css` can follow the Shell theme; the libadwaita variables and the `gtk-interface-contrast` binding do not exist here |
| Screen reader | awkward | St has some accessibility support [unconfirmed how much for custom actors]. Likely worse than GTK. Test with Orca before claiming parity |

## 3. The helper: a session D-Bus service

### 3.1 Shape

- A new headless binary, working name `vexos-ai-helper`, on `vexos-ai-core` and
  `vexos-ai-viewmodel`. It owns the `Session`, `Conversation` and `Composer`
  exactly as `Ui` does today in `main.rs` [repo], with no window.
- **Bus name: `io.github.victorytek.VexosAssistant`** (today's panel app id), as
  a `gio::Application`-style owner that also implements
  `org.freedesktop.Application`. Reason: notification clicks activate
  `app.diagnose-*` on that name, and `checks.module` already asserts the service
  `Name=` equals the desktop id [repo: "Draft flow hardening" 3]. Keeping the
  name keeps that path.
- **Private interface** `io.github.victorytek.VexosAssistant.Helper1` at
  `/io/github/victorytek/VexosAssistant/Helper`, defined below.
- **Activation file:** `share/dbus-1/services/io.github.victorytek.VexosAssistant.service`
  with `Name=` that name and `Exec=` the wrapped helper (as the module rewrites
  it today so the pinned agents are on `PATH`) [repo]. `SystemdService=` exists in
  the D-Bus service-file format (spec revision 0.30 note **[src: D-Bus spec
  revision list]**); whether to use it, and how the NixOS D-Bus implementation
  treats it, is [unconfirmed]. The plain `Exec=` form is what works today.
- **The GApplication surface is part of the attack surface.** A
  `gio::Application` owner exports `org.gtk.Actions` (every registered action)
  and `org.freedesktop.Application` to every bus client with **no sender
  check** [unconfirmed in detail; that is how GApplication works as I know it].
  So the helper registers **only** `diagnose-unit` and `diagnose-coredump` (the
  existing, re-validated, draft-only actions [repo `notify.rs`]); no action may
  reach `Send`, `Decide`, a mode, the agent or a command; remote command-line
  handling is off. A source scan asserts the list of registered action names.
  The `toggle` action is not registered by the helper; the GTK fallback keeps
  its own.
- The extension keeps its own name `…VexosAssistant.Extension` only if something
  still needs it; the GTK-button reason for it goes away.

### 3.2 Methods (the whole call surface)

| Method | Arguments | What the helper does | Notes |
|---|---|---|---|
| `Open()` | none | Starts the saved agent's session (`saved_agent`). Returns the current snapshot, with a **session generation number** | Starting is not sending (existing rule) |
| `Close()` | none | `Conversation::close(&session)` (denies what is pending; it also sets `State::Ended`, permanently, [repo `conversation.rs:832-845`]), drops the session, helper exits (section 5) | Same as GTK Close |
| `Edited(text: s)` | UTF-8, same limit as `Send` | `Composer::edited(text)`, so Rust always knows what is in the input (the GTK panel calls it on every buffer change [repo `main.rs`]). `Composer::offer` decides "place or ask" by whether the input is blank, so a stale copy would let a draft overwrite typed text | helper limit 10 per second; the extension coalesces to at most 5 (3.4) |
| `Send(text: s, via: s)` | UTF-8, **≤ 72 KiB proposed** (reject over); `via` ∈ `"button"`, `"enter"` | `Composer::edited(text)` then `Composer::submit(Via::…, conversation.can_send())`. Only an `Outgoing` that `submit` returns is passed to `Conversation::send` and `Session::prompt` | **The only user send.** `via` is the caller's word, so "Enter does nothing on a pending draft" holds in Rust only because `Composer` refuses `Via::EnterKey` on a pending draft; a caller that says `"button"` for Enter is a caller that clicked Send as far as Rust can tell (see 3A.4) |
| `Decide(session: t, request: t, choice: s)` | `choice` ∈ exactly `"approve-once"`, `"deny-once"`; `session` must equal the current generation | `Conversation::click(request, Decision, &session)` | Section 3A. Any other string, or a stale generation: refused |
| `Shown(session: t, request: t)` | | marks a card as drawn (3A.4) | sender-checked like the rest |
| `CheckUsage()` | none | `Conversation::check_usage()` → the constant `/usage` | **The only fixed send.** No argument |
| `Cancel()` | none | cancel the turn; core answers pending requests `cancelled` | |
| `DraftChoice(choice: s)` | `"replace"`→`Placement::Replace`, `"append"`→`Placement::Append`, `"keep"`→`Placement::Cancel` | `Composer::resolve(Placement)` | for the placement question |
| `DraftDiscard()` | none | `Composer::discard()` | |

Text that Rust puts back into the input (a draft placed, a refused message
returned by `Composer::restore`/`take_unsent`, a discard restoring the user's
text) reaches the extension's entry in the `Draft`/`State` signals as the new
input text; the extension sets it and must not echo it back as an edit (the
`syncing` guard's job in the GTK panel [repo]).

There is **no** method to set a mode, an agent, a working directory, a command,
a path, a permission option other than the two, or to read a file. Adding one is
a contract change and needs this note, the bus tests and a reviewer.

### 3.3 Signals (display only, sent to the Shell only)

`State(s json)`, `Item(u32 index, s json)`, `Usage(s json)`, `Draft(s json)`,
`Ended(s reason)`. Contents are the view-model's own output, already safe to
show: `Run{text, style}` lists from `markdown()`, `Proposal`/`DiffLine`,
`ApprovalCard{request, title, proposal, buttons, answer}`, `UsageButton` label,
`starting_note`, `mode_label`, `Empty`, composer banner state. **The extension
never parses markdown, builds a diff or decides what a button is.** Buttons
arrive as a list; the only values are the two choices.

- Signals are sent **unicast to the Shell's unique name**, not broadcast, so
  another client with a match rule does not receive them. This reduces but does
  not remove exposure (3A.3).
- Streaming text is coalesced by the helper to at most ~20 updates per second so
  the Shell re-lays out text rarely. [proposed]

### 3.4 Size limits and validation (authoritative in the helper, re-checked in JS)

| Item | Limit [proposed, tune in Phase 0] |
|---|---|
| `Send` / `Edited` text | 72 KiB, valid UTF-8 (D-Bus strings must be). Made up of: a log excerpt of up to `MAX_BYTES` 16 KiB, the preamble and markers [repo `diagnose.rs`], and an allowance of about 48 KiB for the user's own text under "Add below" (the allowance is a choice, not derived). Past the limit, `Send` is refused and the text stays in the entry. An over-limit `Edited` is not dropped silently: it sets the composer's copy to "input not blank, unknown", so `Composer::offer` asks instead of placing a draft over it. The extension flushes any pending (coalesced) `Edited` before it applies a `Draft` signal, and does not overwrite an entry that changed since its last `Edited`. A test must show the largest possible draft is accepted |
| One agent reply item sent to the Shell | 64 KiB, then truncated with a visible "… shortened" mark (replies only) |
| Runs per item | 4000 |
| One signal's payload | 256 KiB |
| Items kept in the Shell's list | last 200; older ones are dropped from view (not from Rust) |
| **Approval card content** (title, command, plan, diff) | **There is no existing view-model size cap on these** [repo `proposal.rs`: the only limit, `DIFF_LIMIT`, switches the diff algorithm]. A card whose content is over the display cap (proposed 64 KiB) is **never shown cut**: Rust denies it at once (the card is created already answered "denied: too large to review") and never offers `approve-once`; `Decide` refuses it. Otherwise an agent could pad a command past the cut and hide its tail. |
| Method rate | `Send` ≤ 2, `Decide` ≤ 5, `Edited` ≤ 10, `Shown` ≤ 10, `DraftChoice` ≤ 2 per second, else refused. The extension stays well under: `Edited` at most 5 per second. A rate-refused `Edited` is treated like an over-limit one (the composer's copy becomes "not blank, unknown"). The first `Shown` for a pending card not yet marked shown is exempt from the limit (so a retry still works); repeats count against it |
| Card cap measured | **after** escaping (4), since the cap applies to what is shown |
| Unknown request id, wrong state, over-limit arguments | refused with a D-Bus error; never panics |

### 3.5 How every existing safety rule stays in Rust

| Rule | Where it lives today | In the new design | What crosses the bus |
|---|---|---|---|
| Pinned modes, allow-list, reset on a bad report | core `Session` | unchanged | the confirmed mode string, read-only (`Event::Mode` → `mode_label`) |
| Options filter (only approve-once / deny-once reach the UI) | core `Options` | unchanged | the buttons list (≤ 2 values) |
| Deny by default; timeout 600 s; cancel; drop denies | core | unchanged; the only paths that answer are `Decide` (approve or deny), and deny-only paths: `Close`, `Cancel`, the Shell vanishing, the core's timeout, an over-cap card (3.4) | `TimedOut` shown as card state |
| `check_usage` is the only fixed send | `Conversation::check_usage`, constant `USAGE_COMMAND` | same call behind `CheckUsage()`; launch usage (`take_launch_usage`) stays inside the helper | no text |
| `Composer` is the only user send | `Composer::submit`, `Outgoing` private field | same; `Send` calls `edited` + `submit` | the user's text, which Rust then treats as typed text |
| Draft gating (Enter does nothing on a pending draft; placement question; Discard restores) | `Composer` | same; the extension only forwards | draft state in `Draft` |
| Click is the only decide | `Conversation::click` | `Decide` is the only caller | two fixed strings + a request id |
| Close denies pending | `Conversation::close` | `Close()`, the Shell's bus name vanishing (3A.2) and extension `disable()` on screen lock (5) | none |
| Panel cannot rebuild / launch anything | scan `the_panel_has_no_way_to_rebuild_the_system` [repo] | same scan on the helper crate | n/a |
| Source scans of the send paths | `scan_tests.rs` over GTK `main.rs` [repo] | **rewritten for the helper's source** (one `.prompt(` call, `Outgoing(` only in `submit` and `check_usage`, one `Subprocess`-style spawn with fixed argv). The scans on the GTK file go with it, or stay while the fallback exists | n/a |

**What the extension must never be allowed to do** (and the helper must be
unable to be told to do):

1. Choose or change the agent, a mode, the working directory or any command line.
2. Answer a request with anything but the two choices, or answer a request that
   is not pending now.
3. Send text on any path except `Send` (typed text) and `CheckUsage` (the constant).
4. Read or write files, run a process, open a network connection or use `eval`.
5. Make the helper do anything on its own initiative (the helper acts only on
   the calls above and on agent events).
6. Learn more than the display state (no tokens, no environment, no raw agent
   protocol).

## 3A. Other programs running as the user

### 3A.1 Today, for comparison [repo]

- GTK clicks are in-process: nothing on the bus can approve or send.
- Bus-reachable today: `org.freedesktop.Application` on the panel's name. Any
  session-bus client can call `toggle` (it shows or hides the window, and a
  cold call opens it and starts an agent session, though it sends nothing
  [repo `main.rs`]), `diagnose-unit(s)` and
  `diagnose-coredump((us))` (validated again; they only run the fixed read-only
  collectors and put a **draft** in the input, never send) [repo: `notify.rs`].
- A same-user process can already do much more outside the bus: run `claude`
  itself, edit the user's files, ptrace or inject into the panel unless the
  kernel's ptrace policy forbids it [unconfirmed which policy this system has],
  and possibly click GTK buttons through the accessibility bus [unconfirmed
  whether it is enabled here]. The managed deny rules stay in force in all of
  these because they are enforced inside the agent.

### 3A.2 The new design: how the helper checks the caller

Every method call carries a sender, the caller's **unique** bus name, which the
bus sets and a client cannot forge [unconfirmed in text this session; standard
bus behaviour]. The helper:

1. Looks up the owner of `org.gnome.Shell` with `GetNameOwner` at startup and
   keeps it current from `NameOwnerChanged`.
2. For every `Open`, `Close`, `Edited`, `Send`, `Decide`, `Shown`, `CheckUsage`,
   `Cancel`, `DraftChoice`, `DraftDiscard`: refuses unless the sender equals that owner
   (error `org.freedesktop.DBus.Error.AccessDenied`). Not a well-known name
   other than `org.gnome.Shell` is trusted: our own `…Extension` name is not
   proof, since any process can take it while the extension is off.
3. Optionally also asks `GetConnectionCredentials` for the caller and checks
   the uid is its own and the process id is that of `gnome-shell` (the pid is
   weak evidence; defence in depth only) [unconfirmed field details; the D-Bus
   spec page was truncated when fetched].
4. If the Shell's name loses its owner (Shell restarted or crashed), treats the
   panel as closed: **denies everything pending and ends the session**, as
   `Close()` would. A new owner must `Open()` again.
5. Sends all signals to that owner only.

This assumes the extension's calls come from the Shell's own connection, so the
sender is `org.gnome.Shell`'s owner. That is how GJS extensions use
`Gio.DBus.session` as far as I know, but **it must be checked in Phase 0**
[unconfirmed].

### 3A.3 What a hostile same-user program can do

*A fake helper.* The analysis so far has every caller talking **to** the real
helper. While the helper is not running (before activation, after it exits, after
a crash) any same-user process can own `io.github.victorytek.VexosAssistant`. The
extension would then send the user's typed prompts to it and draw its invented
cards and replies, and a private socket does not help if the other end is the
wrong program. Mitigations: before trusting a name owner, the extension asks the
bus for its credentials (`GetConnectionCredentials`: uid, process id) and checks
the process is the helper started from the expected path [unconfirmed what is
checkable from JS], or, stronger, the helper is started as a systemd user unit
and the extension pins that unit's unique name. Without such a check this is a
real gap and it belongs in the verdict below. It is also why the extension
should call `Open()` only when the name owner is the activated helper.

*With the check working:* it cannot call `Send` or `Decide`. It can still call
`org.freedesktop.Application` actions that exist today (the `diagnose-*` ones,
which only create drafts; `toggle` goes away with the GTK window). It may be
able to **read** conversation text with `dbus-monitor`-style monitoring of the
session bus, if the bus permits monitoring by the same user (usually yes
[unconfirmed]). Unicast signals do not stop a monitor. **This is a new
confidentiality exposure: today the conversation never touches the bus.**

*If the check fails or is bypassed* (a bug, or an attacker who injects code into
`gnome-shell`, or a hostile or buggy **other extension**, which runs in the same
process and the same connection):

- `Send(text)`: arbitrary prompts to the agent, ≤ 72 KiB (3.4), rate-limited. The agent
  still asks before acting and the managed deny rules still apply. A same-user
  attacker could already run the agent directly, so this adds little for them;
  what it adds is *using the user's already-open session and its context*.
- `Decide(id, "approve-once")`: approves **one currently pending request**,
  with the pending request's content, which the user may not have seen when
  the caller never drew it (they cannot change what the request does;
  only the agent could). `Decide` cannot approve a request that is not pending,
  cannot pick "always", and cannot approve something the managed policy denies
  (that is refused before any request exists). The realistic harm is approving
  a request the user was about to read or deny.
- Nothing else: no mode, agent, path or command can be set (3.2).

### 3A.4 `Decide` hardening, and the verdict

`Decide` accepts a call only if **all** hold: sender is the Shell (3A.2);
`choice` is exactly one of the two strings; `request` is the id of a card that is
**pending right now** in `Conversation` (otherwise `NoSuchCard` / `AlreadyAnswered`
as in `click`); the card offered that choice. Proposed additions, all in Rust,
each cheap:

- The extension reports `Shown(session, request)` after it has drawn the card;
  `Decide` is refused for a card not reported shown.
- A short minimum age (for example 400 ms) between `Shown` and `approve-once`
  being accepted. `deny-once` has no delay. The extension keeps the Approve button insensitive until 400 ms after the `Shown` **reply** returns (the helper's clock starts when it receives `Shown`, which is earlier), and shows a notice on any refused `Decide`, so a refused click is never a silent no-op.
- `Decide` is refused while the panel is reported closed.
- A **session generation** number comes with every snapshot and every `Decide`
  and is refused on mismatch. Request ids restart at 1 in each new session
  [repo `session.rs`], so without it a late `Decide(1, …)` aimed at a card of an
  ended session could answer request 1 of the next one. Within one session each
  request answers once.

**Be plain about what `Shown` and the minimum age are:** they come from the same
party that calls `Decide`. They guard against bugs, click-through and scripts
that do not know the protocol. They do **not** stop a hostile extension, which
can call `Shown`, wait 400 ms and approve. Only the sender check and the Shell's
integrity do that. The same goes for `via`: Rust refuses Enter on a pending
draft, but it cannot know whether the caller really saw an Enter key.

**Is this weaker than the GTK panel for approvals? Yes.**

- *Where the loss is real:* (1) the trust boundary grows from one process to
  the Shell **and every extension enabled in it**; one bug in any of them can
  approve. (2) Approval is now a message, so anything that can pass the sender
  check can send it. (3) A same-user process can pose as the helper while the real one is not
  running (3A.3), unless the extension checks who owns the name. (4) What the
  user *sees* is drawn by JavaScript from
  display data; a rendering bug or a hostile extension could show different
  text from what is pending. The helper cannot verify what was drawn.
  (5) The conversation, including command and diff text, is on the bus.
- *How much:* against **an attacker already running as the user**, it is a
  moderate reduction: that attacker could already do anything the user can, and
  the agent's own deny rules still hold, but the old design offered no bus path
  to approve at all. Against **remote or unprivileged** attackers it changes
  nothing (they are not on the user's session bus). Against **a display bug**,
  it is a new risk class that the GTK panel had in a smaller form.
- *What can be bought back:* the sender check, the `Shown`/minimum-age rules and
  unicast signals above; and, if the cost is worth it, **replacing the bus
  methods with a private channel**: `Open()` returns one end of a socketpair as a
  unix file descriptor (D-Bus passes file descriptors), and all later traffic
  runs on that socket. Then other bus clients have no `Send`/`Decide` entry point
  or signal stream to attack, and nothing is visible to bus monitors; only
  `Open` is on the bus, behind the same sender check. This costs a framing
  protocol and fd handling in GJS [unconfirmed that GJS and the chosen Rust bus
  crate handle it without trouble]. **Decision for you (section 11).**

## 4. Security: untrusted agent text inside the privileged gnome-shell process

Agent output, file contents shown in diffs, command lines and error text are
untrusted. They would be rendered by code running as part of the compositor.

| Risk | Mitigation | Remaining |
|---|---|---|
| **Markup injection** (Pango markup or St markup in text) | Every label is set with plain `set_text`; `use_markup` is never enabled (a source scan in node: no `set_markup`, `use_markup`, `markup`); styling comes from the boolean style flags in `Run`, mapped by the extension to fixed style classes or to attribute ranges it builds itself | A mistake in the mapping code; scan test catches the obvious forms only |
| **Huge text, huge layout** | caps in 3.4, enforced in Rust and again in JS (drop an oversized payload and show a notice); coalesced updates; last 200 items only | Pango layout of a 64 KiB label may cost tens of ms [unconfirmed]; measure in Phase 0 |
| **Freeze** (JS runs on the Shell's one main thread; a long layout or loop stops the desktop) | no loops over unbounded input; all D-Bus calls async with the existing cancellable; no synchronous call to the helper, ever; run counts capped | There is no watchdog I know of [unconfirmed]; a freeze lasts until the work ends |
| **Crash** (a bug in St/Clutter/Pango with odd input, e.g. invalid UTF-8, huge widths, control characters) | In **replies**, Rust strips C0/C1 controls except newline and tab. In **approval cards** nothing is stripped (the shown command must be the command that runs): control characters and bidirectional overrides (U+202A to U+202E, U+2066 to U+2069) are shown visibly, escaped (for example `\u{202e}`), by Rust before sending. A literal backslash in card content is escaped too (`\\`), so an escape sequence in a command cannot be mistaken for a real escaped character; the escapes are drawn in a distinct style as well. The card cap (3.4) is measured after escaping. D-Bus strings are valid UTF-8; line length is capped for replies | On Wayland the Shell is the compositor, so **a Shell crash ends the whole graphical session** (apps die) [unconfirmed in the docs I read; widely reported]. This is the worst outcome and the main reason for the gate |
| **Exceptions** | every signal handler and key handler in `try/catch`; on error the panel shows "Display error, close and reopen" and stops applying updates. **Handlers never log payloads** (agent text, commands, diffs, the user's text) to the journal, in exceptions or D-Bus errors; log the error kind only | A swallowed error may leave stale content on screen, which matters for approval cards (see 3A.4) |
| **Code execution** | no `eval`, no `Function`, no dynamic import from data; no `GLib.spawn`/`Gio.Subprocess`, no file or network API in the panel code (node source scans, like the existing "no subprocess, file, clipboard, notification or eval" scan in `layout.test.mjs` [repo]) | The clipboard **Copy** button needs `St.Clipboard`; that is the one deliberate exception and sends only text the user already sees |
| **Clickable content** | no links, no images, no URI handlers; link targets stay as plain text (as `markdown.rs` already does) | none |
| **Spoofed UI** (agent text that looks like an approval card) | cards are drawn only from `ApprovalCard` data; replies are in a different container with a different style; no agent text is used as a style class or widget name | The look is only as good as the theme; an agent can still *write* "Approve this" in a reply |

**How a bug would show to the user:** a blank or stale panel; a "Display error"
notice; an entry in `journalctl --user -u gnome-shell` / the Shell log; at worst
the desktop freezes for seconds or the session ends. Compare today: a GTK panel
bug takes down only the panel window.

## 5. Lifecycle and resource use

**Target:** with the panel closed and idle, the only thing running for this
feature is the extension (a few signal handlers, hidden actors).

| | A: Close = end session | B: Close, then a short idle (proposed 120 s) |
|---|---|---|
| Idle cost after close | none | helper + agent run for up to 120 s |
| Reopen within the window | new session; agent start paid again (unmeasured) | instant, conversation intact |
| Reopen later | new session | new session |
| Pending approval at close | denied (existing `Conversation::close`) | denied at close (an approval is never kept alive while hidden) |
| What is lost | conversation, usage line (re-asked at launch) | the same, after the idle |
| **Code needed** | **none**: `Conversation::close` ends the conversation for good (`State::Ended`, nothing leaves it) [repo], so `Close()` drops the session and the helper exits | **a new view-model method** that denies pending requests but keeps the conversation usable, with its own tests and mutation checks. This adds a fourth view-model change (section 6) |
| Complexity | lowest | a timer, a new state, a view-model change |
| Matches today? | no: today hide keeps everything and any pending approval stays pending until the process ends [repo: "Extension"] | no, for the same reason |

**Recommendation: A (close ends the session, the helper exits at once) for the
first release; adopt B only if Phase 0 measures the real agent start as slow
enough to matter.** A needs no view-model change, is the simplest to prove safe,
and leaves nothing running. The panel itself still opens instantly; only the
agent's start is paid again, and "Starting…" covers it. **Do not recommend a
resident helper.** Both options are behaviour changes from today's hide.

Resume rules (A): `Open()` always starts a fresh session (a draft the helper holds is kept, see the watchers paragraph). If the Shell restarts or its
name vanishes, the helper ends the session (3A.2, rule 4). If the helper
crashes, the extension sees its name vanish, shows "The assistant stopped" and
offers reopen; pending requests were already denied by the dropped session.

**Screen lock.** The Shell disables and re-enables the extension on every lock
[repo: "Docked window" review]; the Shell's bus name does not vanish. Specified
behaviour: `disable()` calls `Close()` (so pending approvals are denied and the
session ends), pops the modal grab and destroys the panel actors; `enable()`
starts closed. A test in Phase 3 covers it.

**Notification watchers (failed units, coredumps) conflict with "nothing running
while idle".** Today they live in the panel and `app.hold()` it [repo:
`notify.rs`]. They are off by default. Options: (a) move them to a separate small
user unit that exists only when the setting is on, and keep the panel helper
non-resident; (b) do the watching in the extension: rejected, it adds timers,
system-bus calls and file monitoring to the privileged process; (c) drop them
for the Shell-native mode. Recommended: (a), after the panel works. A
notification click then D-Bus-activates the helper (name kept, 3.1), whose
`diagnose-*` action collects the log and holds a draft; when the extension sees
the name appear it calls `Open()` and finds the draft waiting in the snapshot.
That pull model avoids a signal sent before anyone listens. The extension
**shows the panel whenever it calls `Open()` after an activation**, so a
session never runs hidden; and `Open()` keeps a draft the helper already holds
(it starts a fresh session, not a fresh draft). Any same-user process can
already open the panel today through `toggle`, so this is not a new exposure.
[untested design]

## 6. Reuse versus replace

| Keep as is | Why |
|---|---|
| `vexos-ai-core` (all) | headless, tested, owns every pin and the deny-by-default |
| `vexos-ai-viewmodel` (existing modules) | `Conversation`, `Composer`, `Proposal`, `markdown`, `usage`, `diagnose`, `redact`, notify rules; their tests and mutation checks carry over. **Not entirely unchanged**: three additions are needed, each with tests: (1) a card over the display cap is created already answered "denied: too large to review", so the request is denied at once and no buttons are offered (3.4); (2) escaping of control and bidi characters in card content (4); (3) **block structure for St**: `markdown()` returns a flat `Run{text, Style{bold, italic, code, heading}}` list with no block boundaries, and inline code and code blocks share `code: true`, so "a code block is one label" and "headings are block labels" would otherwise need JS to guess blocks from the runs, which breaks "the extension never parses". A block-level output (paragraph, heading, list item, code block, table) is needed in Phase 1, before Phase 2 draws replies (or Phase 2 draws plain text with no code-block styling). Option B in section 5 would add a fourth |
| `window.rs` (`WindowMode`, `docked_layout`) | only for the GTK fallback; remove with it |
| `layout.js` `dockedRect` and its node tests | size of the drop-down; `slideStart`/`slideDecision` become unnecessary |
| `checks.policy`, `checks.contract`, `checks.core` | unchanged |

| Thrown away (once the fallback goes) | |
|---|---|
| `crates/vexos-ai-panel/src/main.rs` (1086 lines), `style.rs`, `style.css`, GTK parts of `notify.rs`, `scan_tests.rs` as written | replaced by the helper's scans |
| `extension.js` window-tracking, `_park`, `_armSlide`, `_reveal`, the `window-mode` setting, `nix/panel.nix`'s GTK wrapping | no window |

| New | |
|---|---|
| `crates/vexos-ai-helper` | D-Bus server, serialisation of display state (needs `serde` derives on view-model types), sender check, an exit when no `Open()` arrives soon after activation (so a notification click with the extension off leaves nothing resident; the draft is lost or re-offered by the notification). An idle timer only if option B is adopted (section 5). **A D-Bus crate choice and any new dependency is a Tier 3 decision of its own; not evaluated here** (the panel already uses `gio`, whose D-Bus server API could avoid a new crate [unconfirmed fit]) |
| JS in the extension | panel actors, input handling, the renderer for `Run`/cards, D-Bus client, validators, caps, geometry |
| Tests | node tests for the pure parts; bus tests for the helper; new source scans |

What moves into JavaScript is **drawing and forwarding only**: no parsing of
agent output, no decisions, no state about approvals except "which card I drew".

## 7. Without the extension: keep a GTK fallback, or require the extension?

| | Keep the GTK window as fallback | Require the extension |
|---|---|---|
| Works on non-GNOME, a Shell that breaks the extension, or a GNOME major not yet supported | yes | no: no assistant at all |
| Code kept | two front ends on one helper: GTK `main.rs`, `style.*`, scans, `nix/panel.nix`; every new feature built twice; two sets of tests | one front end |
| Safety surface | two to review; the GTK one keeps in-process approval | one; approvals only via the bus |
| Risk after a GNOME update (Shell internals change) | fallback still works | **assistant unusable until the extension is fixed** (ddterm needed fixes for 49 [repo, spec source 9]) |
| The Windows plan | the portable core/view-model is untouched either way | same |
| Cost | high ongoing | low ongoing, high exposure |

**Recommendation: keep the GTK window as the fallback until the Shell-native
panel has run for a few GNOME point releases, frozen (no new features), then
decide.** A front end that is the only way to reach the assistant, on an
unstable API, is a poor single point of failure. The fallback would run on the
**same helper** (GTK talks to it through the same interface, or links the same
library in-process); it must not re-implement safety logic. If it is kept
in-process (as today), it remains the stronger approval path and can be the
choice of a user who wants that.

## 8. Testing

- **JS unit tests (node):** extract everything pure into modules with no
  `gi://` imports, as `layout.js` already is: payload validators and caps, the
  `Run` → style-descriptor mapping, button-list checks, geometry, slide
  numbers. Run under `node --test` in `checks.extension`, on built files, as
  today [repo]. Source scans for the forbidden calls (3, 4).
- **Glue that touches St/Clutter/Main** cannot run in node. `gjs` could run
  non-UI code, but `St` needs the Shell's libraries and likely a display
  [unconfirmed]; plan on the nested shell instead.
- **Nested headless shell** (the harness described in "Extension" and "Docked
  window" [repo]): load the extension with a **fake helper** (a small program on
  the private bus that implements `Helper1` with canned data and records calls).
  Drive with `Eval`/`Screenshot` over the private bus; assert: the actor is
  mapped, rows count, a card's buttons exist, `Decide` was called with exactly
  the two strings, `Send` carried the typed text, a hostile payload (oversized,
  markup, control characters, invalid UTF-8 refused by D-Bus) is capped or
  refused.
- **Proving instant open and the slide with frames.** "Refinements" admits the
  earlier check read a property, not pixels [repo]. Instead: set
  `St.Settings.get().slow_down_factor` to a large value, take N screenshots at
  fixed intervals, and also log per frame the actor's `y`/`translation_y` from
  the stage's paint signal; check the first screenshot after the key event
  already shows the panel started (instant) and that `y` is monotonic across
  frames (smooth). For "instant", also log the time from the key handler to the
  first paint with the panel mapped, and require that **no D-Bus reply is awaited
  before `show()`**. The harness failed last time because the repo is on a
  Windows drive under WSL and could not create a Wayland socket [repo]; run it
  on a Linux filesystem (the VM).
- **Helper tests:** the fake-ACP-agent tests that exist for the core and
  view-model, plus bus tests: wrong sender refused for every method; `Decide`
  with a non-pending id, an unknown choice, `"approve-always"`, a card not
  `Shown`, under the minimum age; `Send` over the limit; Shell name vanishing
  denies everything pending; exit when no `Open()` follows activation (idle expiry only under option B); new scans for the send paths.
  Mutation checks as for earlier rounds.
- **Cannot be tested off a real screen:** perceived smoothness at the real
  refresh rate and on real GPUs; variable refresh; HiDPI and multi-monitor
  placement; IME composition; screen reader behaviour; keyboard grab behaviour
  with the user's own shortcuts and other extensions; a Shell freeze under a
  slow machine; how it looks to the eye.

## 9. Migration in phases (each independently shippable and reversible)

Reversibility rule: a new setting `ui` (`window` default, `shell`), so any phase
is undone by flipping it. The extension and helper code is untouched in `window`
mode; **the shared view-model additions (section 6) also apply to the GTK
panel**, as an accepted change: block structure (Phase 1), and the card cap with
control-character escaping (Phase 1, with tests and mutation checks), which make
the GTK panel safer too.

| Phase | Work | Ships as | Undo |
|---|---|---|---|
| **0 (smallest, proves the idea)** | In the extension only: a **static** panel drawn in St, placed below `panelBox` as Looking Glass does, `pushModal`, slide by `y`, a multi-line growing `St.Entry`, a scroll view with fake rows, one fake approval card with two buttons, a Copy button. A **fake helper** implemented in JS (no Rust, no bus) returns canned snapshots. Setting `ui=shell` shows it; default unchanged | an off-by-default experiment | set `ui=window`; delete the file |
| 0 re-runs | the nested-shell frame test (section 8) on a Linux filesystem; the real agent start time (never measured); the shortcut-under-grab question; keyboard focus on card buttons; sender identity of extension D-Bus calls (3A.2); a 64 KiB label's layout time; IME, if a real session is available | notes in the spike results | none |
| **Gate** | proceed only if the input, the grab and the frame test pass | | |
| 1 | `vexos-ai-helper` with the bus interface and sender check, fake-agent and bus tests; **no front end uses it yet**; GTK panel code unchanged, but it gains the card cap and escaping from the shared view-model | package + module option off by default | not installed |
| 2 | Extension draws from the helper behind `ui=shell`: Open, Close, Send, Cancel, CheckUsage, read-only. Approvals are not wired yet, so any request would only time out and be denied; `ui=shell` stays experimental and is not offered for real use | experiment | `ui=window` |
| 3 | Approvals, draft flow, usage line, notification click path, lock/`disable()` closes, close-denies, Shell-vanishes-denies, all scans and bus tests | `ui=shell` usable | `ui=window` |
| 4 | Decide the default; freeze or retire the GTK panel (section 7) | default flips | flip back while the fallback exists |

Nothing in phases 0 to 2 touches `nix/policy.nix`, `checks.policy`, the core's
safety behaviour or the contract. Phase 1 adds a package and module option and
therefore needs `checks.module` cases (no extension enabled by the module, the
service `Name=` rule) as in earlier rounds.

## 10. Effects on other plans

| Plan / problem | Effect |
|---|---|
| **Launch speed** | The panel appears at once; the agent still has to start (unmeasured with real agents). "Starting…" stays. A short idle (option B in section 5) would make a quick reopen instant, at the cost of a view-model change |
| **The slide** | The map-effect race, `_park`, the 400 ms fallback and the Wayland resize ack disappear. Replaced by a plain ease, with its own unknowns (grab, chrome placement) |
| **Dock icon** | Removed: no window, so nothing in the dash, Alt+Tab or overview. Today's window is a normal window [repo]; the helper has no window at all |
| **Notifications** | Click path kept by keeping the bus name; watchers must leave the panel process (section 5) |
| **Usage line** | Same Rust parser and label; drawn as an `St.Label`; the contrast measurements must be redone |
| **Windows plan** (`platform_boundary_spec.md`, read to section 2 [repo]) | The GTK panel was already non-portable. A headless helper over core/view-model is closer to portable, but D-Bus is Linux-only: define the display state and the two-choice calls as transport-neutral messages, with D-Bus one binding. Windows still needs its own front end and policy decision |
| **Problems removed** | window placement bugs; slide vs Shell map effect; dock/Alt+Tab presence; background panel process; per-GNOME-release window-API breakage (replaced by Shell-internal-API breakage) |
| **Problems added** | approvals over the bus; the privileged process renders untrusted text; Shell-internal API instability; two things to test (JS, helper); no libadwaita theming |

## 11. Risks and open decisions for you

- **Decision 1: GNOME-only.** The extension is already GNOME Shell 50 only
  (`shell-version ["50"]`) [repo]. Shell-native deepens that: the panel exists
  only on GNOME. *Recommend:* accept it, since the GTK fallback covers other
  desktops for as long as it is kept.
- **Decision 2: helper lifetime.** *Recommend:* option A, end the session and
  exit at close, deny pending, no resident helper (section 5). Option B (120 s
  idle) only if the measured agent start is slow, and it needs a view-model
  change.
- **Decision 3: keep the GTK fallback?** *Recommend:* yes, frozen, until the
  Shell-native panel has proved itself across GNOME point releases (section 7).
- **Decision 4: how much rich display to give up.** Likely losses: inline bold
  or italic inside a paragraph, selectable text with a context menu, undo and
  spell check in the input, libadwaita theming, screen-reader quality. Keep:
  scrolling, code blocks (better), aligned tables, cards, banner, usage line.
  *Recommend:* give up inline emphasis if the attribute route fails, and keep
  everything safety-related.
- **Decision 5: bus methods or a private socket.** *Recommend:* build the bus
  methods with the checks in 3A.4 first; evaluate the socketpair in Phase 1
  once the sender-identity question is answered.
- **Decision 6: watchers.** Separate opt-in unit (recommended), or drop them.
- **Decision 7: the D-Bus implementation crate** (new dependency): a separate
  Tier 3 decision, not made here.
- **Risk: a Shell crash ends the session.** The deepest cost of this design, and
  why Phase 0 is a gate and not a formality.
- **Risk: Shell-internal APIs** (`Main`, `layoutManager`, `pushModal`, St) change
  between GNOME majors with no compatibility promise.

**Do not do:** put a bus method on the helper that sets a mode, agent, path or
command; parse markdown or diffs in JS; build Pango markup from agent text;
accept calls from any sender but the Shell; move the notification watchers into
the Shell; keep a hidden approval pending; run a resident helper.

## Docs consulted

Read this session: GNOME Shell `gnome-50` branch `js/ui/quickSettings.js`,
`main.js` (`pushModal`/`popModal`), `layout.js` (`addChrome`, `addTopChrome`,
`trackChrome`), `boxpointer.js`, `dateMenu.js`, `lookingGlass.js`; the ddterm
README. Repo documents as listed at the top.

Could not verify, so marked [unconfirmed]: `Clutter.Text` and `St.Entry`
properties for Shell 50 (the GJS docs page returned no content); `St.Clipboard`;
Pango attribute constructors in GJS; D-Bus specification text on
`GetNameOwner`, `GetConnectionCredentials`, `NameOwnerChanged`, activation-file
keys (the page was truncated at 100 000 of 205 621 characters); `messageTray.js`
and `overview.js`; whether extension code shares the Shell's bus connection;
IME and accessibility behaviour; ptrace and bus-monitor policy on this system.
The Context7 and nixos MCP servers failed to connect, so nothing here depends
on them, and the locked Shell 50.5 sources in `/nix/store` were not read (no
Nix on this host); the `gnome-50` branch was read instead, which may differ from
the locked 50.5 revision.

## Phase 0 results

Run 2026-10-30. Code: `spike/shell-panel/` (a separate extension,
`vexos-shell-spike@victorytek.github.io`; fake data, no helper, no D-Bus, no
timers; never enabled by anything in the repo; only the nested-shell harness enables it, inside its own shell). Tested in a **headless nested
GNOME Shell 50.5** (1280x800 virtual monitor, private bus, input from virtual
devices). Not tested on a real screen or GPU; the renderer used was not checked
and is probably software. Screenshots and traces are in
`target/shell-spike-run/` (not committed). Manual steps for the VM are in
`spike/shell-panel/README.md`.

| # | Criterion | Result | Evidence |
|---|---|---|---|
| 1 | Open and close | **PASS** (not instant) | `Super+F10` opened and closed it in every one of 5 cycles, and from inside the overview; `Escape` closes. Key to handler 0.3 ms; **key to first paint 58 to 70 ms (median 61), to the first frame that moves 78 to 89 ms (median 79)**. Close: Escape to handler 17 ms (one frame), first moving frame 34 ms. The first paint is layout of the fake content: with the first 64 KiB cap it took 130 to 156 ms (a 64 KiB label cost about 31 ms a pass, measured once by hand, no trace kept), and with the 16 KiB cap 58 to 70 ms. So the open is quick but not "instant", and it grows with content |
| 2 | Slide | **PASS after one fix** | First version: **fail**. The ease started before a 130 ms first layout, so the first visible frame was already 74% down (`y` -106 of -488 to 32). Fix: start the ease in the first painted frame. Now 16 frames per open, `y` -488, -488, -424, -360, -301, ... 22, 30, never falling; gaps 13.9 to 19.3 ms after a first doubled sample (1 to 3 ms); close 14 frames, gaps 15.6 to 17.8 ms, never rising. Slow-motion shots (factor 20): before = bare desktop; early = only the panel's bottom edge and input under the bar; middle = about three quarters down; end = fully down with header, chat, card, input. The top bar stays drawn over the panel both ways. Real-GPU smoothness untested |
| 3 | Multi-line input | **PASS, with limits** (the gate) | Typing, `Shift+Enter` new line, **Enter sends** (chosen), empty or spaces-only Enter does nothing. Box height 31, 50, 69 ... 164 px for 1 to 8 lines, then stays 164 and scrolls with the cursor kept in view; shrinks back to 31 on delete. Paste (`Ctrl+V`, 3 lines and 15,000 chars), `Ctrl+C`/`Ctrl+X`, `Ctrl+A`, `Shift+Left`/`Shift+Home`, typing or Backspace over a selection all work. Chat: wheel scrolls 52 px a notch; it stays at the bottom after a send. Replies: mouse drag and double-click select text; Copy button put the row's displayed text on the clipboard (escaped, cut to the cap, including the "shortened" line, so what Copy should copy is a Phase 1 decision). **Limits:** no undo (`Ctrl+Z` does nothing); no context menu; **selected text in a reply is unreadable** (white on light, my selection colours did not apply to `St.Label`); **Clutter's text buffer holds at most 65,535 bytes** (not characters), so a 1 MiB paste keeps 65,534 characters and 30,000 three-byte characters keep 21,844, silently, which makes the 72 KiB limit and its "too long" notice unreachable. Bug found and fixed: after a paste the box did not grow, because `actor.width` returns the natural width while a relayout is pending. **Risk, untested:** Enter is handled in the entry's `key-press-event` before `Clutter.Text`'s own key handling, where an input method filters keys, so an IME user pressing Enter to commit a composition may send the message instead. IME, spell check, other layouts untested |
| 4 | Keyboard grab | **PASS** | `pushModal` with `Shell.ActionMode.POPUP` and the keybinding bound for NORMAL, OVERVIEW and POPUP: the shortcut closed the panel while it was open. With a real app (GNOME Text Editor) in the nested shell: `aa` typed before opening, `zz` typed while open went to the panel input, `bb` after closing went to the app (it shows `aabb`). `focus_window` never changed; stage key focus after closing equals the value before opening; `Main.modalCount` 1 open, 0 closed; action mode 128 open, 1 closed. Works the same inside the overview. Side effect: the shortcut also fires while a Shell popup menu is open (not tried). Clicks outside the panel while open: not tried |
| 5 | Scrolling and size caps | **PASS after changing the cap** | **The note's 64 KiB per message failed.** A 64 KiB label painted only its first part (blank after about 4,000 px) and its "shortened" mark was lost, because the Clutter text buffer holds 65,535 bytes. Probes: labels of 14,469 and 22,389 characters painted to the end; 32,889 and more lost the tail, whatever their height (cause not confirmed). **Cap now: 16 KiB per message (after escaping, never inside a character), 2,000 lines, last 200 rows, input 8 lines then scroll.** Overflow is shown as `… shortened: 190670 units not shown` as the last line (units are UTF-16 units of the escaped text, not original characters) (screenshot checked, tail pixels painted); the code block's tail also paints. The 207 KB message: build 10 ms, never froze the shell; 1 MiB paste kept the shell answering in 11 ms; with 65,534 characters in the input a key press takes 12 to 14 ms; 250 rows added in 46 ms (200 kept). Wide table line wraps by character, so nothing overflows sideways |
| 6 | No dock icon, window entry or Alt+Tab entry | **PASS** | With the panel open or closed: window actors 1 (the app only), Alt+Tab list and `Shell.AppSystem.get_running()` identical. The panel is one actor in `uiGroup` placed just before `panelBox`. Overview screenshot: the dash shows its favourites and only Text Editor running, nothing for the spike; the panel can be opened inside the overview |
| 7 | Failure containment | **PASS** | Threw on purpose in a button handler, a key handler and an input handler: each time the panel was removed, the Shell kept answering, `modalCount` 0, action mode NORMAL, key focus restored, no leftover actor, and it opened again. The log has the error name only (`SpikeTestError`, `TypeError`, `RangeError`); two secret message strings I threw never appeared in it. Disabling the extension while open, and in the middle of the slide: actor gone, grab popped, the stage handler (connected before) disconnected, shortcut gone, re-enable works. **Not covered:** a crash in native St/Clutter/Pango code (no `try` can catch it; that is still the end of the session) |
| 8 | Idle cost | **PASS** (by inspection) | Closed, the extension instance holds the `Gio.Settings` for its keybinding and inert leftovers (`_rect`, a flag, and the last `_trace` of up to 600 frames); no actor, grab or stage handler. Source scan (a node test): no timeout, idle, `Meta` later, polling, I/O, D-Bus or markup; the only signal handlers are on actors it creates, plus one stage `after-paint` handler that exists only while a slide runs. Not measured with a profiler (wake-ups, CPU) |

**Not tested** (all need a real session or a person): smoothness at the real
refresh rate and on a real GPU (the first-paint cost may differ); HiDPI and
multi-monitor placement; IME composition, spell check, other keyboard layouts;
screen reader; clicks outside the panel while it is open; other extensions
running together with it; a fullscreen window behind it; screen lock with the
panel open; the shortcut conflicting with the user's own bindings; the sender
identity of an extension's D-Bus calls and everything else about the helper
(Phase 1, not part of this spike). Unit tests cover only the pure logic and the
source scans (40 tests; the 30 mutations in `spike/shell-panel/tests/mutate.py` were all caught); the Shell glue was checked
only by the nested-shell runs above.

**Changes to what this note assumed.** `Clutter.Text` `single_line_mode`,
`line_wrap`, `selectable`, `reactive` and `max_length`, `position_to_coords`,
`St.ScrollView.vadjustment` and `St.Clipboard.set_text(CLIPBOARD, text)` exist
and work in 50.5 (the note marked them [unconfirmed]). Not as assumed: 64 KiB
per message and 72 KiB per send are too big for a `St.Label` or the entry
(about 16 KiB shown, about 64 KiB typed); the ease must start after the first
painted frame; selection colours need their own styling; there is no undo.

**Recommendation: continue, to Phase 1 under conditions.** The gate passed: the
growing multi-line input works, selection and copy work, the slide is smooth once
it starts after the first frame, and the shortcut and focus survive the grab.
Errors and disabling are contained. Conditions before building more: (1) cut
the display to about 16 KiB per message (split or page longer replies) and the
send limit to below the 64 KiB the entry can hold, and detect a truncated paste;
(2) decide whether about 80 ms to open is acceptable (it comes from laying out
the content; keeping the widgets alive between opens would remove it but
conflicts with option A in section 5); (3) accept no undo and no context menu in
the input, or find a fix, and settle Enter-to-send against input methods (the
main risk to the gate; test Enter while composing in the VM); (4) run the manual list in `spike/shell-panel/README.md`
on a real screen in the VM first (smoothness, IME, grab with real shortcuts and
extensions, lock), because everything above is from a headless shell. The main
cost in section 11 is unchanged: a native crash in the Shell ends the session,
and nothing here reduces that.
