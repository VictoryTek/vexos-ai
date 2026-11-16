// The top-bar icon and its drop-down: the assistant's quick panel.
//
// Left click opens the panel (hero, plan limits, starter tiles). Right click
// shows or hides the chat window. Middle click asks the usage service for fresh
// numbers. The icon turns urgent when a limit is at 90% or more.
//
// A thin client, like the rest of the extension: it draws what usage_store.js
// reads, and every action is a call in bus.js or one of the callbacks it is
// given. It starts nothing, sends no text, and reads no credentials.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';

import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {STARTERS} from './starters.js';
import {UsageStore} from './usage_store.js';
import {displayWindows, formatCountdown, isUrgent, percentText, refreshDelaySeconds,
    shouldRefreshOnOpen, statusLine, URGENT_PERCENT} from './usage_view.js';

const USAGE_KEY = 'usage-enabled';
const ICON_FILE = 'icons/vexos-assistant-symbolic.svg';
// While the panel is open, countdowns are redrawn this often.
const OPEN_TICK_SECONDS = 30;

// A rounded track with a fill that grows to a percentage.
const Meter = GObject.registerClass(
class VexosMeter extends St.Widget {
    _init() {
        super._init({
            style_class: 'vexos-meter',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            layout_manager: new Clutter.BinLayout(),
        });
        this._percent = 0;
        this._fill = new St.Widget({
            style_class: 'vexos-meter-fill',
            x_align: Clutter.ActorAlign.START,
            y_expand: true,
        });
        this.add_child(this._fill);
        this.connect('notify::allocation', () => this._layout());
    }

    set(percent, urgent) {
        this._percent = Math.min(100, Math.max(0, percent));
        this._fill.style_class = urgent ? 'vexos-meter-fill vexos-urgent' : 'vexos-meter-fill';
        this._layout();
    }

    _layout() {
        const width = this.get_width();
        if (width <= 0)
            return;
        const target = Math.round(width * this._percent / 100);
        this._fill.ease({
            width: target,
            duration: 160,
            mode: Clutter.AnimationMode.EASE_OUT_CUBIC,
        });
    }
});

const label = (text, styleClass) => new St.Label({
    text,
    style_class: styleClass,
    y_align: Clutter.ActorAlign.CENTER,
});

const iconButton = (iconName, name, styleClass, onClicked) => {
    const button = new St.Button({
        style_class: styleClass,
        can_focus: true,
        accessible_name: name,
        child: new St.Icon({icon_name: iconName, icon_size: 16}),
    });
    button.connect('clicked', onClicked);
    return button;
};

export const AssistantButton = GObject.registerClass(
class VexosAssistantButton extends PanelMenu.Button {
    _init({path, settings, bus, onToggleWindow}) {
        super._init(0.5, 'VexOS Assistant', false);
        this._settings = settings;
        this._bus = bus;
        this._onToggleWindow = onToggleWindow;
        this._timer = 0;
        this._tick = 0;
        this._iconFile = `${path}/${ICON_FILE}`;

        this._icon = new St.Icon({
            gicon: Gio.icon_new_for_string(this._iconFile),
            style_class: 'system-status-icon vexos-icon',
        });
        this.add_child(this._icon);

        // Shell 50 opens the menu from a click gesture that accepts any button.
        // Left stays the menu's; right and middle are ours, as gestures of
        // their own that the menu's does not see.
        this._clickGesture?.set_required_button(Clutter.BUTTON_PRIMARY);
        this._addGesture(Clutter.BUTTON_SECONDARY, () => {
            this.menu.close();
            this._onToggleWindow();
        });
        this._addGesture(Clutter.BUTTON_MIDDLE, () => {
            if (this._enabled())
                this._bus.refreshUsage(true);
        });

        this._build();
        // A new record is new information for the refresh timer too.
        this._store = new UsageStore(() => {
            this._render();
            this._schedule();
        });
        this._settingsId = settings.connect(`changed::${USAGE_KEY}`, () => {
            this._render();
            this._schedule();
        });
        this.menu.connect('open-state-changed', (_menu, open) => this._opened(open));
        this._render();
        this._schedule();
        // One request at login, so the icon and the numbers are not 15 minutes late.
        if (this._enabled())
            this._bus.refreshUsage(false);
    }

    _addGesture(button, onRecognize) {
        const gesture = new Clutter.ClickGesture({required_button: button});
        gesture.connect('recognize', onRecognize);
        this.add_action(gesture);
    }

    destroy() {
        if (this._timer)
            GLib.source_remove(this._timer);
        if (this._tick)
            GLib.source_remove(this._tick);
        this._timer = this._tick = 0;
        if (this._settingsId)
            this._settings.disconnect(this._settingsId);
        this._settingsId = 0;
        this._store?.destroy();
        this._store = null;
        super.destroy();
    }

    _enabled() {
        try {
            return this._settings.get_boolean(USAGE_KEY);
        } catch {
            return false;
        }
    }

    _build() {
        const item = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        this._root = new St.BoxLayout({
            orientation: Clutter.Orientation.VERTICAL,
            style_class: 'vexos-popup',
            x_expand: true,
        });
        item.add_child(this._root);
        this.menu.addMenuItem(item);

        // Hero: the mark, the name and plan, and the two quick actions.
        const hero = new St.BoxLayout({style_class: 'vexos-hero', x_expand: true});
        hero.add_child(new St.Icon({
            gicon: Gio.icon_new_for_string(this._iconFile),
            icon_size: 32,
            style_class: 'vexos-hero-icon',
            y_align: Clutter.ActorAlign.CENTER,
        }));
        const names = new St.BoxLayout({
            orientation: Clutter.Orientation.VERTICAL,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        names.add_child(label('Assistant', 'vexos-title'));
        this._subtitle = label('Claude Code', 'vexos-subtitle');
        names.add_child(this._subtitle);
        hero.add_child(names);
        hero.add_child(iconButton('list-add-symbolic', 'New chat', 'vexos-hero-button', () => {
            this.menu.close();
            this._bus.newSession();
        }));
        hero.add_child(iconButton('utilities-terminal-symbolic', 'Open in a terminal', 'vexos-hero-button', () => {
            this.menu.close();
            this._bus.terminal();
        }));
        this._root.add_child(hero);
        this._root.add_child(new St.Widget({style_class: 'vexos-separator'}));

        // Plan limits: rebuilt on every render.
        this._usage = new St.BoxLayout({
            orientation: Clutter.Orientation.VERTICAL,
            style_class: 'vexos-usage',
            x_expand: true,
        });
        this._root.add_child(this._usage);
        this._root.add_child(new St.Widget({style_class: 'vexos-separator'}));

        // Starter tiles.
        this._root.add_child(label('START WITH A TASK', 'vexos-section'));
        const tiles = new St.BoxLayout({style_class: 'vexos-tiles', x_expand: true});
        for (const starter of STARTERS) {
            const tile = new St.Button({
                style_class: 'vexos-tile',
                can_focus: true,
                x_expand: true,
                accessible_name: starter.title,
            });
            const body = new St.BoxLayout({
                orientation: Clutter.Orientation.VERTICAL,
                x_align: Clutter.ActorAlign.CENTER,
            });
            body.add_child(new St.Icon({
                icon_name: starter.icon,
                icon_size: 20,
                x_align: Clutter.ActorAlign.CENTER,
                style_class: 'vexos-tile-icon',
            }));
            const text = label(starter.title, 'vexos-tile-label');
            text.clutter_text.line_wrap = true;
            text.x_align = Clutter.ActorAlign.CENTER;
            body.add_child(text);
            tile.set_child(body);
            tile.connect('clicked', () => {
                this.menu.close();
                this._bus.draft(starter.id);
            });
            tiles.add_child(tile);
        }
        this._root.add_child(tiles);
    }

    _render() {
        if (!this._store)
            return;
        const now = Date.now();
        const record = this._store.record;
        const enabled = this._enabled();
        const urgent = enabled && isUrgent(record, now);

        this._icon.style_class = urgent
            ? 'system-status-icon vexos-icon vexos-urgent'
            : 'system-status-icon vexos-icon';
        this._subtitle.text = enabled && record?.plan ? `Claude Code · ${record.plan}` : 'Claude Code';

        this._usage.destroy_all_children();
        if (!enabled) {
            this._renderOptIn();
            return;
        }
        if (!record) {
            this._usage.add_child(label('Checking your limits…', 'vexos-note'));
            this._usage.add_child(this._offLink());
            return;
        }
        for (const w of displayWindows(record, now))
            this._usage.add_child(this._row(w, now));
        const line = statusLine(record, now);
        if (line)
            this._usage.add_child(this._note(line));
        this._usage.add_child(this._offLink());
    }

    _row(w, now) {
        const row = new St.BoxLayout({style_class: 'vexos-limit', x_expand: true});
        const urgent = w.percent >= URGENT_PERCENT;
        row.add_child(label(w.label, 'vexos-limit-label'));
        const meter = new Meter();
        meter.set(w.percent, urgent);
        row.add_child(meter);
        row.add_child(label(percentText(w.percent), urgent ? 'vexos-limit-percent vexos-urgent' : 'vexos-limit-percent'));
        row.add_child(label(formatCountdown(w.resetsAt, now), 'vexos-limit-reset'));
        return row;
    }

    _note(text) {
        const note = label(text, 'vexos-note');
        note.clutter_text.line_wrap = true;
        return note;
    }

    // Plan limits are off until the user says so: the service reads Claude's
    // sign-in only after this switch.
    _renderOptIn() {
        this._usage.add_child(label('PLAN LIMITS', 'vexos-section'));
        this._usage.add_child(this._note(
            "See how much of your Claude plan is used. This reads Claude Code's saved sign-in and asks Anthropic for your limits."));
        const on = new St.Button({
            label: 'Show plan limits',
            style_class: 'vexos-action',
            can_focus: true,
            x_align: Clutter.ActorAlign.START,
        });
        on.connect('clicked', () => {
            this._settings.set_boolean(USAGE_KEY, true);
            // Written out first: the service reads the setting itself.
            Gio.Settings.sync();
            this._bus.refreshUsage(true);
        });
        this._usage.add_child(on);
    }

    _offLink() {
        const off = new St.Button({
            label: 'Turn off plan limits',
            style_class: 'vexos-link',
            can_focus: true,
            x_align: Clutter.ActorAlign.START,
        });
        off.connect('clicked', () => {
            this._settings.set_boolean(USAGE_KEY, false);
            Gio.Settings.sync();
            this._bus.refreshUsage(true);
        });
        return off;
    }

    _opened(open) {
        if (this._tick) {
            GLib.source_remove(this._tick);
            this._tick = 0;
        }
        if (!open)
            return;
        this._store.reload();
        if (shouldRefreshOnOpen(this._store.record, Date.now(), this._enabled()))
            this._bus.refreshUsage(false);
        // Countdowns move while the panel is open.
        this._tick = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, OPEN_TICK_SECONDS, () => {
            this._render();
            return GLib.SOURCE_CONTINUE;
        });
    }

    // The next request: every 15 minutes, 3 near a limit, 30 s after nothing
    // could be reached. Nothing runs while plan limits are off.
    _schedule() {
        if (this._timer) {
            GLib.source_remove(this._timer);
            this._timer = 0;
        }
        if (!this._enabled())
            return;
        const seconds = refreshDelaySeconds(this._store?.record ?? null, Date.now());
        this._timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, seconds, () => {
            this._timer = 0;
            if (this._enabled())
                this._bus.refreshUsage(false);
            this._store?.reload();
            this._schedule();
            return GLib.SOURCE_REMOVE;
        });
    }
});
