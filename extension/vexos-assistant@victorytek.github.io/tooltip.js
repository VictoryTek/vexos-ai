// A hover hint for the panel's buttons, like the labels the dash shows.
//
// St has no tooltips of its own. One label is shown at a time, under the widget
// the pointer rests on after a short delay, and hidden when the pointer leaves,
// when the widget is pressed, when the panel closes or when it is destroyed. The
// text is a fixed string given by the caller; nothing here reads or sends data.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

export const TOOLTIP_DELAY_MS = 400;
const GAP = 6;

export class Tooltips {
    constructor() {
        this._label = null;
        this._timer = 0;
    }

    // Makes `actor` track the pointer and show `text` when it rests there.
    attach(actor, text) {
        actor.track_hover = true;
        actor.connect('notify::hover', () => {
            if (actor.hover)
                this._arm(actor, text);
            else
                this.hide();
        });
        actor.connect('button-press-event', () => {
            this.hide();
            return Clutter.EVENT_PROPAGATE;
        });
        actor.connect('destroy', () => this.hide());
    }

    _arm(actor, text) {
        this.hide();
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, TOOLTIP_DELAY_MS, () => {
            this._timer = 0;
            this._show(actor, text);
            return GLib.SOURCE_REMOVE;
        });
    }

    _show(actor, text) {
        if (!actor.get_stage())
            return;
        const label = new St.Label({style_class: 'vexos-tooltip', text, opacity: 0});
        label.clutter_text.line_wrap = true;
        label.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
        Main.layoutManager.uiGroup.add_child(label);
        this._label = label;

        const [ax, ay] = actor.get_transformed_position();
        const [aw, ah] = actor.get_transformed_size();
        const monitor = Main.layoutManager.findMonitorForActor(actor) ??
            Main.layoutManager.primaryMonitor;
        const [, width] = label.get_preferred_width(-1);
        const x = Math.min(Math.max(ax + (aw - width) / 2, monitor.x + GAP),
            monitor.x + monitor.width - width - GAP);
        label.set_position(Math.round(x), Math.round(ay + ah + GAP));
        label.ease({
            opacity: 255,
            duration: 120,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
    }

    hide() {
        if (this._timer) {
            GLib.source_remove(this._timer);
            this._timer = 0;
        }
        if (this._label) {
            this._label.destroy();
            this._label = null;
        }
    }
}
