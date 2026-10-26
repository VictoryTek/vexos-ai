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
// moved once, when it is shown, and the slide animates the actor's
// translation only, so no position is read back while it runs.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';

const APP_ID = 'io.github.victorytek.VexosAssistant';
const APP_PATH = '/io/github/victorytek/VexosAssistant';
const SHORTCUT = 'toggle-assistant';
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

        this._indicator = new AssistantIndicator(() => this._toggle());
        Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator);

        this._settings = this.getSettings();
        Main.wm.addKeybinding(SHORTCUT, this._settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW, () => this._toggle());

        this._createdId = global.display.connect('window-created',
            (_display, window) => this._watch(window));
        // Already open: follow it for the tile, but leave it where it is.
        for (const actor of global.get_window_actors())
            this._track(actor.meta_window, false);
        this._sync();
    }

    disable() {
        Main.wm.removeKeybinding(SHORTCUT);
        if (this._createdId)
            global.display.disconnect(this._createdId);
        this._createdId = 0;
        for (const window of [...this._pending.keys()])
            this._unwatch(window);
        this._pending = null;
        for (const window of [...this._windows.keys()])
            this._untrack(window, true);
        this._windows = null;
        this._cancellable.cancel();
        this._cancellable = null;
        this._indicator.destroy();
        this._indicator = null;
        this._settings = null;
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

    // Once per window, when it is first shown: one move to the top centre of
    // its monitor's work area, then a slide down from above the screen edge.
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
            const frame = window.get_frame_rect();
            const monitor = window.get_monitor();
            const area = window.get_work_area_for_monitor(monitor);
            const screen = global.display.get_monitor_geometry(monitor);
            const x = area.x + Math.round((area.width - frame.width) / 2);
            window.move_frame(false, x, area.y);
            window.make_above();
            window.stick();
            entry.raised = true;

            actor.remove_all_transitions();
            actor.translation_y = -(frame.height + area.y - screen.y);
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
