// The top-bar icon and its drop-down: the assistant's quick panel.
//
// Left click opens the panel (hero, each agent's plan limits, starter tiles). Right click
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
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {STARTERS} from './starters.js';
import {Tooltips} from './tooltip.js';
import {UsageStore} from './usage_store.js';
import {AGENT_ICONS, displayWindows, formatCountdown, isUrgent, pendingDelayMs, pendingLine, percentText,
    refreshDelaySeconds, shouldRefreshOnOpen, shownSources, statusLine, URGENT_PERCENT} from './usage_view.js';

const USAGE_KEY = 'usage-enabled';
const ICON_FILE = 'icons/vexos-assistant-symbolic.svg';
// While the panel is open, countdowns are redrawn this often.
const OPEN_TICK_SECONDS = 30;
// Opening the panel warms the app at most this often.
const WARM_EVERY_MS = 60 * 1000;

// A rounded track with a fill that grows to a percentage.
const Meter = GObject.registerClass(
class VexosMeter extends St.Widget {
    _init() {
        super._init({
            style_class: 'vexos-meter',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            // A fixed layout puts the fill at the track's left edge by position,
            // not by alignment, so it can only grow from the left.
            layout_manager: new Clutter.FixedLayout(),
        });
        this._percent = 0;
        this._fill = new St.Widget({style_class: 'vexos-meter-fill'});
        this._fill.set_position(0, 0);
        this.add_child(this._fill);
        this.connect('notify::allocation', () => this._layout());
    }

    set(percent, urgent, agent) {
        this._percent = Math.min(100, Math.max(0, percent));
        this._fill.style_class = `vexos-meter-fill vexos-fill-${agent}${urgent ? ' vexos-urgent' : ''}`;
        this._layout();
    }

    _layout() {
        const width = this.get_width();
        if (width <= 0)
            return;
        // Set, not eased: easing a width animates the allocation, and a new
        // fill grew out from its middle instead of from the left edge.
        this._fill.set_position(0, 0);
        this._fill.set_size(Math.round(width * this._percent / 100), this.get_height());
    }
});

const label = (text, styleClass) => new St.Label({
    text,
    style_class: styleClass,
    y_align: Clutter.ActorAlign.CENTER,
});

// Every button tracks the pointer, so its :hover style applies.
const iconButton = (iconName, name, styleClass, onClicked) => {
    const button = new St.Button({
        style_class: styleClass,
        can_focus: true,
        track_hover: true,
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
        // Plan limits are on and no record has arrived: since when (ms), and
        // the timer that turns "checking" into "no answer".
        this._waitingSince = 0;
        this._pendingTimer = 0;
        this._warmedAt = 0;
        this._tips = new Tooltips();
        this._path = path;
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
        if (this._pendingTimer)
            GLib.source_remove(this._pendingTimer);
        this._timer = this._tick = this._pendingTimer = 0;
        this._tips?.hide();
        this._tips = null;
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
        // The menu keeps the Shell's own surface; only its content is styled.
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
            y_align: Clutter.ActorAlign.CENTER,
        }));
        const names = new St.BoxLayout({
            orientation: Clutter.Orientation.VERTICAL,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        names.add_child(label('Assistant', 'vexos-title'));
        this._subtitle = label('AI help for this system', 'vexos-subtitle');
        names.add_child(this._subtitle);
        hero.add_child(names);
        const newChat = iconButton('list-add-symbolic', 'New chat', 'icon-button vexos-hero-button', () => {
            this.menu.close();
            this._bus.newSession();
        });
        this._tips.attach(newChat, 'New chat');
        hero.add_child(newChat);
        const terminal = iconButton('utilities-terminal-symbolic', 'Open in a terminal', 'icon-button vexos-hero-button', () => {
            this.menu.close();
            this._bus.terminal();
        });
        this._tips.attach(terminal, 'Open in a terminal');
        hero.add_child(terminal);
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
                style_class: 'vexos-tile vexos-tile-' + starter.id,
                can_focus: true,
                track_hover: true,
                x_expand: true,
                accessible_name: starter.title,
            });
            this._tips.attach(tile, 'Starts a draft in the chat. Nothing is sent until you click Send.');
            const body = new St.BoxLayout({
                orientation: Clutter.Orientation.VERTICAL,
                x_align: Clutter.ActorAlign.CENTER,
            });
            body.add_child(new St.Bin({
                style_class: 'vexos-tile-badge',
                x_align: Clutter.ActorAlign.CENTER,
                child: new St.Icon({icon_name: starter.icon, icon_size: 18, style_class: 'vexos-tile-icon'}),
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

        this._tips.hide();
        this._usage.destroy_all_children();
        if (!enabled) {
            this._waiting(false);
            this._renderOff();
            return;
        }
        // On, but nothing usable yet: no file, or one the service wrote while
        // the switch was off. Say "checking", and after a while say it is not
        // answering, so a service that never ran is not an endless spinner.
        if (!record || !record.enabled) {
            this._waiting(true);
            this._usage.add_child(this._note(pendingLine(now, this._waitingSince)));
            this._usage.add_child(this._offLink());
            return;
        }
        this._waiting(false);
        for (const source of shownSources(record))
            this._usage.add_child(this._section(source, now));
        this._usage.add_child(this._offLink());
    }

    // Starts or stops the wait for a record, and schedules the redraw that
    // turns "checking" into "no answer".
    _waiting(on) {
        if (!on) {
            this._waitingSince = 0;
            if (this._pendingTimer) {
                GLib.source_remove(this._pendingTimer);
                this._pendingTimer = 0;
            }
            return;
        }
        if (!this._waitingSince)
            this._waitingSince = Date.now();
        if (this._pendingTimer)
            return;
        // Once the wait is over the sentence is already showing: no more timers.
        const left = pendingDelayMs(Date.now(), this._waitingSince);
        if (left <= 0)
            return;
        this._pendingTimer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, left, () => {
            this._pendingTimer = 0;
            this._render();
            return GLib.SOURCE_REMOVE;
        });
    }

    // One agent: its name and plan, its meters, and the sentence its situation
    // needs. Claude's always; Codex's when the service found Codex.
    _section({agent, name, data}, now) {
        const section = new St.BoxLayout({
            orientation: Clutter.Orientation.VERTICAL,
            style_class: `vexos-agent vexos-agent-${agent}`,
            x_expand: true,
        });
        const head = new St.BoxLayout({x_expand: true, style_class: 'vexos-agent-head'});
        head.add_child(new St.Icon({
            gicon: Gio.icon_new_for_string(`${this._path}/${AGENT_ICONS[agent]}`),
            icon_size: 18,
            style_class: 'vexos-agent-logo',
            y_align: Clutter.ActorAlign.CENTER,
        }));
        head.add_child(label(name, 'vexos-agent-name'));
        if (data.plan)
            head.add_child(label(data.plan, 'vexos-agent-plan'));
        section.add_child(head);
        for (const w of displayWindows(data, now))
            section.add_child(this._row(w, now, agent));
        const line = statusLine(data, now, agent);
        if (line)
            section.add_child(this._note(line));
        return section;
    }

    _row(w, now, agent) {
        const row = new St.BoxLayout({style_class: 'vexos-limit', x_expand: true});
        const urgent = w.percent >= URGENT_PERCENT;
        row.add_child(label(w.label, 'vexos-limit-label'));
        const meter = new Meter();
        meter.set(w.percent, urgent, agent);
        row.add_child(meter);
        row.add_child(label(percentText(w.percent), urgent ? 'vexos-limit-percent vexos-urgent' : 'vexos-limit-percent'));
        row.add_child(label(formatCountdown(w.resetsAt, now), 'vexos-limit-reset'));
        return row;
    }

    // Wraps inside the popup's fixed width, never widens it.
    _note(text) {
        const note = label(text, 'vexos-note');
        note.clutter_text.line_wrap = true;
        note.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
        return note;
    }

    // Plan limits are on unless the user turned them off here: this is the way
    // back. The service reads Claude's sign-in only while the switch is on.
    _renderOff() {
        this._usage.add_child(this._note('Plan limits are off.'));
        const on = this._link('Turn on plan limits',
            "Shows your Claude and Codex plan limits. Reads Claude Code's saved sign-in and asks Anthropic; if Codex is installed, asks Codex.");
        on.connect('clicked', () => {
            this._settings.set_boolean(USAGE_KEY, true);
            // Written out first: the service reads the setting itself.
            Gio.Settings.sync();
            this._bus.refreshUsage(true);
        });
        this._usage.add_child(on);
    }

    _offLink() {
        const off = this._link('Turn off plan limits',
            "Stops reading Claude Code's sign-in. You can turn it back on here.");
        off.connect('clicked', () => {
            this._settings.set_boolean(USAGE_KEY, false);
            Gio.Settings.sync();
            this._bus.refreshUsage(true);
        });
        return off;
    }

    // A quiet text button with a hover hint.
    _link(text, hint) {
        const link = new St.Button({
            label: text,
            style_class: 'vexos-link',
            can_focus: true,
            track_hover: true,
            x_align: Clutter.ActorAlign.START,
        });
        this._tips.attach(link, hint);
        return link;
    }

    _opened(open) {
        if (this._tick) {
            GLib.source_remove(this._tick);
            this._tick = 0;
        }
        this._tips.hide();
        if (!open)
            return;
        // Someone who opens this panel will often open the chat next: have the
        // agent start now, hidden. At most once a minute.
        const now = Date.now();
        if (now - this._warmedAt > WARM_EVERY_MS) {
            this._warmedAt = now;
            this._bus.prewarm();
        }
        // A fresh look at an empty panel gets its full wait again.
        if (this._waitingSince)
            this._waitingSince = Date.now();
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
