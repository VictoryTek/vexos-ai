// Phase 0 spike (shell_native_panel_spec.md): a drop-down chat panel drawn in
// the Shell with St and Clutter, in the style of Looking Glass, with FAKE data.
// Throwaway. No helper, no D-Bus, no agent, no files, no network, no timers.
//
// Closed, this holds the keybinding and nothing else. Open, it adds actors, a
// modal grab and, during a slide only, one stage 'after-paint' handler for the
// frame trace. Every handler runs through _guard: an error is logged by name
// only and tears the panel down (actors, grab, handler).
//
// All text is set with plain `.text`; markup is never enabled.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {CARD, CONVERSATION} from './fake.js';
import * as Logic from './logic.js';

const SHORTCUT = 'toggle-panel';
const PANEL_NAME = 'vexos-shell-spike-panel';

const Panel = GObject.registerClass(
class SpikePanel extends St.BoxLayout {
    constructor(onKey) {
        super({name: PANEL_NAME, style_class: 'spike-panel', vertical: true, reactive: true});
        this._onKey = onKey;
    }

    vfunc_key_press_event(event) {
        return this._onKey(event) ?? super.vfunc_key_press_event(event);
    }
});

export default class SpikeExtension extends Extension {
    enable() {
        this._state = 'closed';
        this._panel = null;
        this._grab = null;
        this._stageId = 0;
        this._trace = null;
        this._settings = this.getSettings();
        Main.wm.addKeybinding(SHORTCUT, this._settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW | Shell.ActionMode.POPUP,
            this._guard(() => this._toggle()));
    }

    disable() {
        Main.wm.removeKeybinding(SHORTCUT);
        this._teardown();
        this._settings = null;
        this._trace = null;
    }

    // Wraps a handler: on any error, log the error's name (never its message,
    // which could carry text) and remove the panel. Key handlers must return a
    // value, so the wrapper stops the event.
    _guard(fn) {
        return (...args) => {
            try {
                return fn(...args);
            } catch (e) {
                console.warn(`vexos-shell-spike: handler error: ${e?.name ?? 'Error'}`);
                this._teardown();
                return Clutter.EVENT_STOP;
            }
        };
    }

    _toggle() {
        if (this._state === 'open')
            this._close();
        else
            this._open();
    }

    _open() {
        const keyAt = GLib.get_monotonic_time();
        if (this._state === 'closing') {
            // Reverse a close in progress: same panel, same grab.
            this._state = 'open';
            this._slide(this._rect.targetY, null);
            return;
        }
        if (this._state !== 'closed')
            return;
        const bar = Main.layoutManager.panelBox;
        this._rect = Logic.panelRect(Main.layoutManager.primaryMonitor, bar.height);
        const panel = this._build(this._rect);
        this._panel = panel;
        Main.uiGroup.add_child(panel);
        Main.uiGroup.set_child_below_sibling(panel, bar);
        // POPUP keeps our own shortcut alive under the grab (it is bound for it).
        this._grab = Main.pushModal(panel, {actionMode: Shell.ActionMode.POPUP});
        this._state = 'open';
        panel.show();
        this._trace = {keyAt, shownAt: GLib.get_monotonic_time(), firstPaintAt: 0, frames: [],
            what: 'open'};
        this._slide(this._rect.targetY, this._trace, true);
        this._entry.grab_key_focus();
        this._stick = true;
    }

    _close() {
        if (this._state !== 'open')
            return;
        this._state = 'closing';
        this._trace = {keyAt: GLib.get_monotonic_time(), shownAt: 0, firstPaintAt: 0, frames: [],
            what: 'close'};
        this._slide(this._rect.hiddenY, this._trace);
    }

    // Eases the panel's y. The frame trace listens to the stage only while the
    // ease runs. With `afterFirstPaint` the ease starts in the first painted
    // frame, so a slow first layout does not use up the slide. A finished close
    // removes everything; an interrupted one (reversed) does not, since
    // onComplete runs only for a finished ease.
    _slide(y, trace, afterFirstPaint = false) {
        const panel = this._panel;
        panel.remove_all_transitions();
        this._unwatchStage();
        const start = () => panel.ease({
            y,
            duration: Logic.SLIDE_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onStopped: this._guard(() => this._unwatchStage()),
            onComplete: this._guard(() => {
                if (this._state === 'closing')
                    this._teardown();
            }),
        });
        if (trace) {
            this._stageId = global.stage.connect('after-paint', this._guard(() => {
                if (!this._panel || !trace.frames)
                    return;
                const now = GLib.get_monotonic_time();
                if (!trace.firstPaintAt) {
                    trace.firstPaintAt = now;
                    if (afterFirstPaint)
                        start();
                }
                if (trace.frames.length < 600)
                    trace.frames.push([now, this._panel.y]);
            }));
        }
        if (!afterFirstPaint || !trace)
            start();
    }

    _unwatchStage() {
        if (!this._stageId)
            return;
        const id = this._stageId;
        this._stageId = 0;
        global.stage.disconnect(id);
    }

    // Idempotent. Pops the grab itself: it must not wait for an ease callback.
    _teardown() {
        const panel = this._panel;
        this._panel = null;
        this._state = 'closed';
        try {
            this._unwatchStage();
        } catch {}
        if (panel)
            panel.remove_all_transitions();
        const grab = this._grab;
        this._grab = null;
        if (grab) {
            try {
                Main.popModal(grab);
            } catch {}
        }
        this._entry = this._chat = this._chatScroll = this._inputScroll = null;
        this._inputBox = this._notice = null;
        if (panel)
            panel.destroy();
    }

    _build(rect) {
        const panel = new Panel(this._guard(event => this._panelKey(event)));
        panel.set({x: rect.x, y: rect.hiddenY, width: rect.width, height: rect.height});

        const header = new St.BoxLayout({style_class: 'spike-header'});
        header.add_child(new St.Label({text: 'Shell panel spike (fake data)', x_expand: true,
            y_align: Clutter.ActorAlign.CENTER, style_class: 'spike-title'}));
        const close = new St.Button({label: 'Close', style_class: 'button spike-button',
            can_focus: true});
        close.connect('clicked', this._guard(() => this._close()));
        header.add_child(close);
        panel.add_child(header);

        this._chatScroll = new St.ScrollView({style_class: 'spike-chat', x_expand: true,
            y_expand: true, overlay_scrollbars: true, enable_mouse_scrolling: true,
            hscrollbar_policy: St.PolicyType.NEVER, vscrollbar_policy: St.PolicyType.AUTOMATIC});
        this._chat = new St.BoxLayout({vertical: true, x_expand: true, style_class: 'spike-rows'});
        this._chatScroll.add_child(this._chat);
        panel.add_child(this._chatScroll);
        // Stay at the bottom after a send or the first open, until the user scrolls.
        this._chatScroll.vadjustment.connect('notify::upper', this._guard(() => {
            const a = this._chatScroll?.vadjustment;
            if (a && this._stick)
                a.value = a.upper - a.page_size;
        }));
        this._chatScroll.connect('scroll-event', this._guard(() => {
            this._stick = false;
            return Clutter.EVENT_PROPAGATE;
        }));

        for (const row of Logic.capRows(CONVERSATION))
            this._addRow(row);
        this._chat.add_child(this._card());

        this._notice = new St.Label({style_class: 'spike-notice', visible: false});
        panel.add_child(this._notice);

        const test = new St.Button({label: 'Throw a test error', style_class: 'button spike-button',
            can_focus: true, x_align: Clutter.ActorAlign.START});
        test.connect('clicked', this._guard(() => {
            const err = new Error('deliberate');
            err.name = 'SpikeTestError';
            throw err;
        }));
        panel.add_child(test);

        this._buildInput(panel);
        return panel;
    }

    _buildInput(panel) {
        this._entry = new St.Entry({style_class: 'spike-entry', can_focus: true, x_expand: true,
            hint_text: 'Enter sends, Shift+Enter adds a line'});
        const ct = this._entry.clutter_text;
        ct.set({single_line_mode: false, line_wrap: true, line_wrap_mode: Pango.WrapMode.WORD_CHAR,
            ellipsize: Pango.EllipsizeMode.NONE, activatable: false,
            max_length: Logic.MAX_INPUT_CHARS});
        // The St.Entry sits in a vertical box (a scrollable) inside a scroll
        // view whose height follows the text up to a cap.
        this._inputBox = new St.BoxLayout({vertical: true, x_expand: true});
        this._inputBox.add_child(this._entry);
        this._inputScroll = new St.ScrollView({style_class: 'spike-input', x_expand: true,
            overlay_scrollbars: true, hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC});
        this._inputScroll.add_child(this._inputBox);
        panel.add_child(this._inputScroll);

        ct.connect('text-changed', this._guard(() => this._inputChanged()));
        ct.connect('cursor-changed', this._guard(() => this._showCursor()));
        ct.connect('key-press-event', this._guard((_actor, event) => this._entryKey(event)));
    }

    _lineHeight() {
        const ct = this._entry.clutter_text;
        const r = ct.position_to_coords(0);
        const h = Array.isArray(r) ? r[r.length - 1] : 0;
        return Number.isFinite(h) && h > 0 ? h : 20;
    }

    // The box is one line to INPUT_MAX_LINES lines high, following the text. This
    // runs from 'text-changed', when a relayout is pending and `actor.width`
    // would return the natural (unwrapped) width, so it uses the last allocated
    // width, and the entry's padding comes from its theme node.
    _inputChanged() {
        const entry = this._entry;
        const width = entry.get_allocation_box().get_width();
        const natural = entry.get_preferred_height(width > 0 ? width : -1)[1];
        const node = entry.get_theme_node();
        const pad = node.get_vertical_padding() +
            node.get_border_width(St.Side.TOP) + node.get_border_width(St.Side.BOTTOM);
        const line = this._lineHeight();
        this._inputScroll.height = Logic.inputHeight(natural, line + pad, line * Logic.INPUT_MAX_LINES + pad);
        this._showCursor();
    }

    // Keeps the cursor's line inside the visible part of the input box. The
    // coordinates are the text's own; the box's include the entry's padding.
    _showCursor() {
        const ct = this._entry.clutter_text;
        const r = ct.position_to_coords(ct.cursor_position);
        if (!Array.isArray(r) || r.length < 4)
            return;
        const [, y, line] = r.slice(r.length - 3);
        const node = this._entry.get_theme_node();
        const top = y + node.get_padding(St.Side.TOP) + node.get_border_width(St.Side.TOP);
        const adj = this._inputScroll.vadjustment;
        adj.value = Logic.scrollToShow(adj.value, adj.page_size, top,
            top + line + node.get_padding(St.Side.BOTTOM) + node.get_border_width(St.Side.BOTTOM));
    }

    _entryKey(event) {
        const sym = event.get_key_symbol();
        if (sym !== Clutter.KEY_Return && sym !== Clutter.KEY_KP_Enter)
            return Clutter.EVENT_PROPAGATE;
        // Shift+Enter: let the text add its newline. Enter alone sends.
        if (event.get_state() & Clutter.ModifierType.SHIFT_MASK)
            return Clutter.EVENT_PROPAGATE;
        this._send();
        return Clutter.EVENT_STOP;
    }

    // "Send" in the spike only adds the text as a row. Nothing leaves the Shell.
    _send() {
        const ct = this._entry.clutter_text;
        const verdict = Logic.inputVerdict(ct.text);
        if (verdict === 'empty')
            return;
        if (verdict === 'too-long') {
            this._notice.text = 'Too long to send (limit 72 KiB). The text stays in the box.';
            this._notice.show();
            return;
        }
        this._notice.hide();
        this._addRow({role: 'you', kind: 'text', text: ct.text});
        ct.text = '';
        this._stick = true;
    }

    // Escape closes; every other key goes on to the focused actor.
    _panelKey(event) {
        if (event.get_key_symbol() === Clutter.KEY_Escape) {
            this._close();
            return Clutter.EVENT_STOP;
        }
        return null;
    }

    _addRow({role, kind, text}) {
        const row = new St.BoxLayout({vertical: true, style_class: `spike-row spike-${role}`});
        row.add_child(new St.Label({text: role === 'you' ? 'You' : 'Assistant',
            style_class: 'spike-role'}));
        const shown = Logic.capText(text);
        const label = new St.Label({text: shown, reactive: true, x_expand: true,
            style_class: kind === 'code' ? 'spike-code' : 'spike-text'});
        label.clutter_text.set({selectable: true, reactive: true, line_wrap: true,
            line_wrap_mode: kind === 'code' ? Pango.WrapMode.CHAR : Pango.WrapMode.WORD_CHAR,
            ellipsize: Pango.EllipsizeMode.NONE});
        row.add_child(label);
        const copy = new St.Button({label: 'Copy', style_class: 'button spike-button',
            can_focus: true, x_align: Clutter.ActorAlign.START});
        copy.connect('clicked', this._guard(() =>
            St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, shown)));
        row.add_child(copy);
        this._chat.add_child(row);
        while (this._chat.get_n_children() > Logic.MAX_ROWS)
            this._chat.get_child_at_index(0).destroy();
    }

    // The fake approval card: its content is shown escaped (control characters
    // and backslashes visible). Its buttons only change the status line.
    _card() {
        const card = new St.BoxLayout({vertical: true, style_class: 'spike-card'});
        card.add_child(new St.Label({text: Logic.visible(CARD.title, {backslash: true}),
            style_class: 'spike-card-title'}));
        card.add_child(new St.Label({text: Logic.visible(CARD.command, {backslash: true}),
            style_class: 'spike-code'}));
        const status = new St.Label({text: 'Pending (fake)'});
        const buttons = new St.BoxLayout({style_class: 'spike-card-buttons'});
        for (const [label, result] of [['Approve once', 'Approved (fake)'], ['Deny once', 'Denied (fake)']]) {
            const b = new St.Button({label, style_class: 'button spike-button', can_focus: true});
            b.connect('clicked', this._guard(() => {
                status.text = result;
            }));
            buttons.add_child(b);
        }
        card.add_child(buttons);
        card.add_child(status);
        return card;
    }
}
