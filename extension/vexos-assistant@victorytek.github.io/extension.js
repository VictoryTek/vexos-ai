// VexOS Assistant: a Quick Settings tile and a shortcut that show or hide the
// assistant window (crates/vexos-ai-panel), sliding it down from the top.
//
// A thin client. It calls the app's `toggle` action by app id over D-Bus,
// which takes no parameter, and places the app's main window. It reads no
// conversation, answers no approval, starts no agent and shows no mode: all of
// that stays in the app. Without this extension the app is a plain centred
// window.
//
// Placement follows assistant_panel_spec.md 5.4 (Mutter has returned stale
// positions after a move, and crashed in move_resize_frame): the window is
// sized and moved once, when it is shown, and the slide animates the actor's
// translation only, so no position is read back while it runs.
//
// The window is "docked" (a drop-down under the top bar, sized by layout.js)
// or "floating" (left to Mutter), by the `window-mode` setting. The app writes
// it from its header button; only the strings "docked" and "floating" mean
// anything here. The extension also owns a D-Bus name while enabled, so the
// app knows to show that button.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';

import {DEFAULT_NORMAL, DOCKED, FLOATING, dockedRect, floatingRect, normalSize, parseMode}
    from './layout.js';

const APP_ID = 'io.github.victorytek.VexosAssistant';
const APP_PATH = '/io/github/victorytek/VexosAssistant';
const EXTENSION_NAME = `${APP_ID}.Extension`;
const SHORTCUT = 'toggle-assistant';
const MODE_KEY = 'window-mode';
const SLIDE_MS = 180;

const AssistantToggle = GObject.registerClass(
class AssistantToggle extends QuickSettings.QuickToggle {
    constructor(onClicked) {
        super({
            title: 'Assistant',
            iconName: 'utilities-terminal-symbolic',
            toggleMode: false,
        });
        this.connect('clicked', onClicked);
    }
});

const AssistantIndicator = GObject.registerClass(
class AssistantIndicator extends QuickSettings.SystemIndicator {
    constructor(onClicked) {
        super();
        this.toggle = new AssistantToggle(onClicked);
        this.quickSettingsItems.push(this.toggle);
    }

    destroy() {
        this.quickSettingsItems.forEach(item => item.destroy());
        super.destroy();
    }
});

export default class AssistantExtension extends Extension {
    enable() {
        this._cancellable = new Gio.Cancellable();
        // Meta.Window -> {ids, placed, raised}
        this._windows = new Map();
        // New windows not identified yet: Meta.Window -> handler ids
        this._pending = new Map();
        // The window's normal size, read once at the first show.
        this._normal = null;

        this._indicator = new AssistantIndicator(() => this._toggle());
        Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator);

        this._settings = this.getSettings();
        Main.wm.addKeybinding(SHORTCUT, this._settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW, () => this._toggle());

        this._createdId = global.display.connect('window-created',
            (_display, window) => this._watch(window));
        this._modeId = this._settings.connect(`changed::${MODE_KEY}`,
            () => this._modeChanged());
        this._nameId = Gio.bus_own_name(Gio.BusType.SESSION, EXTENSION_NAME,
            Gio.BusNameOwnerFlags.NONE, null, null, null);
        // Already open: follow it for the tile, and dock it if the mode is docked.
        for (const actor of global.get_window_actors())
            this._track(actor.meta_window, false);
        this._sync();
    }

    disable() {
        Main.wm.removeKeybinding(SHORTCUT);
        if (this._nameId)
            Gio.bus_unown_name(this._nameId);
        this._nameId = 0;
        if (this._modeId)
            this._settings.disconnect(this._modeId);
        this._modeId = 0;
        if (this._createdId)
            global.display.disconnect(this._createdId);
        this._createdId = 0;
        for (const window of [...this._pending.keys()])
            this._unwatch(window);
        this._pending = null;
        for (const [window, entry] of [...this._windows]) {
            if (entry.raised)
                this._apply(window, entry, FLOATING);
            this._untrack(window, true);
        }
        this._windows = null;
        this._cancellable.cancel();
        this._cancellable = null;
        this._indicator.destroy();
        this._indicator = null;
        this._settings = null;
        this._normal = null;
    }

    // Ask the app to show or hide its window; D-Bus starts it if needed. The
    // action has no parameter, so nothing else can be asked.
    _toggle() {
        Gio.DBus.session.call(APP_ID, APP_PATH, 'org.freedesktop.Application',
            'ActivateAction', new GLib.Variant('(sava{sv})', ['toggle', [], {}]),
            null, Gio.DBusCallFlags.NONE, -1, this._cancellable,
            (connection, result) => {
                try {
                    connection.call_finish(result);
                } catch (e) {
                    if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                        console.warn(`VexOS Assistant: toggle failed: ${e.message}`);
                }
            });
    }

    // The app's main window: its app id, a normal window, and no parent (the
    // app's rebuild terminal is a transient window, and is left alone).
    _isAssistant(window) {
        try {
            const id = window.get_gtk_application_id() ?? window.get_wm_class();
            return id === APP_ID &&
                window.get_window_type() === Meta.WindowType.NORMAL &&
                !window.get_transient_for();
        } catch {
            return false;
        }
    }

    // On Wayland a new window's app id is set after it is created, so a window
    // that can't be identified yet is decided when it is first shown.
    _watch(window) {
        if (this._isAssistant(window)) {
            this._track(window, true);
            return;
        }
        try {
            const ids = [];
            this._pending.set(window, ids);
            ids.push(window.connect('shown', () => {
                this._unwatch(window);
                const entry = this._track(window, true);
                if (entry) {
                    this._place(window, entry);
                    this._sync();
                }
            }));
            ids.push(window.connect('unmanaging', () => this._unwatch(window)));
        } catch {
            this._unwatch(window);
        }
    }

    _unwatch(window) {
        const ids = this._pending?.get(window);
        if (!ids)
            return;
        this._pending.delete(window);
        for (const id of ids) {
            try {
                window.disconnect(id);
            } catch {}
        }
    }

    _track(window, place) {
        if (!window || this._windows.has(window) || !this._isAssistant(window))
            return null;
        const entry = {ids: [], placed: !place, raised: false};
        this._windows.set(window, entry);
        // Open when the extension was enabled (also after a screen lock, which
        // disables it): learn its normal size, and dock it to match the
        // app's look. One move, no slide.
        if (!place) {
            try {
                if (!window.is_hidden() && !window.is_maximized() && !window.is_fullscreen()) {
                    this._remember(window);
                    if (this._mode() === DOCKED)
                        this._apply(window, entry, DOCKED);
                }
            } catch (e) {
                console.warn(`VexOS Assistant: cannot adopt the window: ${e.message}`);
            }
        }
        try {
            entry.ids.push(window.connect('unmanaging', () => this._untrack(window, false)));
            entry.ids.push(window.connect('shown', () => {
                this._place(window, entry);
                this._sync();
            }));
        } catch (e) {
            console.warn(`VexOS Assistant: cannot follow the window: ${e.message}`);
        }
        this._sync();
        return entry;
    }

    _untrack(window, restore) {
        const entry = this._windows?.get(window);
        if (!entry)
            return;
        this._windows.delete(window);
        for (const id of entry.ids) {
            try {
                window.disconnect(id);
            } catch {}
        }
        if (restore) {
            try {
                const actor = window.get_compositor_private();
                if (actor) {
                    actor.remove_all_transitions();
                    actor.translation_y = 0;
                }
                if (entry.raised) {
                    window.unmake_above();
                    window.unstick();
                }
            } catch {}
        }
        this._sync();
    }

    _mode() {
        try {
            return parseMode(this._settings.get_string(MODE_KEY));
        } catch {
            return DOCKED;
        }
    }

    // The setting changed while a window is open: one move to the new mode.
    _modeChanged() {
        const mode = this._mode();
        for (const [window, entry] of this._windows) {
            if (!entry.placed)
                continue;
            this._apply(window, entry, mode);
        }
    }

    // Shown windows only; a hidden one is placed when it is next shown.
    _apply(window, entry, mode) {
        try {
            if (window.is_hidden() || window.is_maximized() || window.is_fullscreen())
                return;
            const actor = window.get_compositor_private();
            const area = window.get_work_area_for_monitor(window.get_monitor());
            if (actor) {
                actor.remove_all_transitions();
                actor.translation_y = 0;
            }
            if (mode === FLOATING) {
                const r = floatingRect(area, this._normal ?? DEFAULT_NORMAL);
                window.move_resize_frame(false, r.x, r.y, r.width, r.height);
                if (entry.raised) {
                    window.unmake_above();
                    window.unstick();
                    entry.raised = false;
                }
            } else {
                const r = dockedRect(area);
                window.move_resize_frame(false, r.x, r.y, r.width, r.height);
                window.make_above();
                window.stick();
                entry.raised = true;
            }
        } catch (e) {
            console.warn(`VexOS Assistant: cannot change the window mode: ${e.message}`);
        }
    }

    // Read once per enable(), before any move by this extension.
    _remember(window) {
        if (this._normal)
            return;
        const area = window.get_work_area_for_monitor(window.get_monitor());
        this._normal = normalSize(window.get_frame_rect(), area);
    }

    // Once per window, when it is first shown: the normal size is remembered
    // (once per enable), then, if docked, one move and resize to the work
    // area's top and a slide down from above the screen edge.
    _place(window, entry) {
        if (entry.placed)
            return;
        entry.placed = true;
        try {
            if (window.is_hidden() || window.is_maximized() || window.is_fullscreen())
                return;
            const actor = window.get_compositor_private();
            if (!actor)
                return;
            // Read once, before the move; never after it.
            this._remember(window);
            if (this._mode() !== DOCKED)
                return;
            const monitor = window.get_monitor();
            const area = window.get_work_area_for_monitor(monitor);
            const screen = global.display.get_monitor_geometry(monitor);
            const r = dockedRect(area);
            window.move_resize_frame(false, r.x, r.y, r.width, r.height);
            window.make_above();
            window.stick();
            entry.raised = true;

            actor.remove_all_transitions();
            actor.translation_y = -(r.height + area.y - screen.y);
            actor.ease({
                translation_y: 0,
                duration: SLIDE_MS,
                mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            });
        } catch (e) {
            console.warn(`VexOS Assistant: cannot place the window: ${e.message}`);
        }
    }

    // The tile is on while the assistant window is open and shown.
    _sync() {
        if (!this._indicator || !this._windows)
            return;
        this._indicator.toggle.checked = [...this._windows.keys()].some(window => {
            try {
                return !window.is_hidden();
            } catch {
                return false;
            }
        });
    }
}
