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
// The slide has to beat Shell's own map effect. For a normal window Shell
// (windowManager.js _mapWindow, async) scales the actor up from its bottom
// edge and fades it in over 150 ms, starting that ease in a promise job that
// runs after 'shown' returns; its _mapWindowDone then resets translation_y to
// 0 and removes all transitions. On Wayland the resize is also only applied
// when the client acks it. So, behind Shell's promise job, the effect is ended
// and the window hidden (opacity 0) and parked above the screen in one turn,
// moved once, and only shown, sliding, when its move has landed (or after a
// short timeout, so it can never stay invisible).
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

import {DEFAULT_NORMAL, DOCKED, FALLBACK_MS, FLOATING, SLIDE_EASING, SLIDE_MS, dockedRect,
    floatingRect, normalSize, parseMode, slideDecision, slideStart} from './layout.js';

const APP_ID = 'io.github.victorytek.VexosAssistant';
const APP_PATH = '/io/github/victorytek/VexosAssistant';
const EXTENSION_NAME = `${APP_ID}.Extension`;
const SHORTCUT = 'toggle-assistant';
const MODE_KEY = 'window-mode';

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
        // Meta.Window -> {ids, placed, raised, slide}
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

    // The app's main window: its app id, a normal window, and no parent (a
    // transient window of the app is left alone).
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
        const entry = {ids: [], placed: !place, raised: false, slide: null};
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
        this._cancelSlide(window, entry);
        if (restore) {
            try {
                const actor = window.get_compositor_private();
                if (actor) {
                    actor.remove_all_transitions();
                    actor.translation_y = 0;
                    actor.opacity = 255;
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
            this._cancelSlide(window, entry);
            if (actor) {
                actor.remove_all_transitions();
                actor.translation_y = 0;
                actor.opacity = 255;
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
    // (once per enable), then, if docked, the window is hidden and parked above
    // the screen edge, moved once to the work area's top, and slid down when
    // the move has landed. Floating never touches opacity or translation.
    _place(window, entry) {
        if (entry.placed)
            return;
        entry.placed = true;
        try {
            if (window.is_hidden() || window.is_maximized() || window.is_fullscreen())
                return;
            if (!window.get_compositor_private())
                return;
            // Read once, before the move; never after it.
            this._remember(window);
            if (this._mode() !== DOCKED)
                return;
            if (Main.overview.visible) {
                // Shell holds its map effect until the overview closes, so a
                // parked actor would be overridden: dock without the slide.
                this._apply(window, entry, DOCKED);
                return;
            }
            // Shell's _mapWindow is async: it has set the actor to scale
            // 0.01 / opacity 0 but starts its ease only in a promise job, which
            // runs after this signal returns. Queue behind it (jobs are FIFO)
            // so the effect is ended and the actor parked in one turn, before
            // any frame.
            Promise.resolve().then(() => this._park(window, entry)).catch(e => {
                console.warn(`VexOS Assistant: cannot place the window: ${e.message}`);
                this._cancelSlide(window, entry);
                this._reveal(this._actor(window));
            });
        } catch (e) {
            console.warn(`VexOS Assistant: cannot place the window: ${e.message}`);
        }
    }

    _actor(window) {
        try {
            return window.get_compositor_private();
        } catch {
            return null;
        }
    }

    // Ends Shell's map effect, hides and parks the actor above the screen, and
    // moves the window once. The landing is waited for in _armSlide.
    _park(window, entry) {
        // Closed, disabled or no longer docked since the signal.
        if (this._windows?.get(window) !== entry || window.is_hidden() ||
            window.is_maximized() || window.is_fullscreen())
            return;
        const actor = window.get_compositor_private();
        if (!actor)
            return;
        const monitor = window.get_monitor();
        const area = window.get_work_area_for_monitor(monitor);
        const screen = global.display.get_monitor_geometry(monitor);
        const r = dockedRect(area);
        const start = slideStart(r.height, area.y, screen.y);

        actor.remove_all_transitions();
        actor.opacity = 0;
        actor.translation_y = start;
        this._armSlide(window, entry, actor, start);

        window.move_resize_frame(false, r.x, r.y, r.width, r.height);
        window.make_above();
        window.stick();
        entry.raised = true;
    }

    // Waits for the move to land (the window reports a new position or size),
    // or for the fallback timer, then shows the window.
    _armSlide(window, entry, actor, start) {
        const slide = {ids: [], timer: 0, idle: 0};
        entry.slide = slide;
        const decide = state => {
            if (entry.slide !== slide)
                return;
            const verdict = slideDecision(state);
            if (verdict === 'wait')
                return;
            this._cancelSlide(window, entry);
            if (verdict === 'slide')
                this._reveal(actor, start);
            else if (verdict === 'show')
                this._reveal(actor);
        };
        const landed = () => {
            // One idle later, so the actor has the new geometry.
            if (slide.idle || entry.slide !== slide)
                return;
            slide.idle = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                slide.idle = 0;
                decide({landed: true, gone: !this._windows?.has(window)});
                return GLib.SOURCE_REMOVE;
            });
        };
        slide.timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, FALLBACK_MS, () => {
            slide.timer = 0;
            decide({timedOut: true, gone: !this._windows?.has(window)});
            return GLib.SOURCE_REMOVE;
        });
        try {
            slide.ids.push(window.connect('position-changed', landed));
            slide.ids.push(window.connect('size-changed', landed));
        } catch {
            decide({timedOut: true, gone: false});
        }
    }

    _cancelSlide(window, entry) {
        const slide = entry?.slide;
        if (!slide)
            return;
        entry.slide = null;
        if (slide.timer)
            GLib.source_remove(slide.timer);
        if (slide.idle)
            GLib.source_remove(slide.idle);
        slide.timer = slide.idle = 0;
        for (const id of slide.ids) {
            try {
                window.disconnect(id);
            } catch {}
        }
    }

    // Makes the parked actor visible: sliding down from `start`, or, with no
    // start, at once in its place.
    _reveal(actor, start = null) {
        try {
            if (!actor)
                return;
            actor.remove_all_transitions();
            actor.opacity = 255;
            if (start === null) {
                actor.translation_y = 0;
                return;
            }
            // Ending a transition can reset the actor: park it again first.
            actor.translation_y = start;
            actor.ease({
                translation_y: 0,
                duration: SLIDE_MS,
                mode: Clutter.AnimationMode[SLIDE_EASING],
            });
        } catch (e) {
            console.warn(`VexOS Assistant: cannot show the window: ${e.message}`);
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
