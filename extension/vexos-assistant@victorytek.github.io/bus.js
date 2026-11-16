// Every D-Bus call the extension makes, in one place, so one test can list them.
//
// All of them are `org.freedesktop.Application.ActivateAction` on an app id:
//   - the assistant app: `new-session`, `terminal`, and `draft` with a starter
//     id (the app maps the id to a fixed prompt and puts it in the input as a
//     draft; nothing is sent until the user clicks Send there)
//   - the usage service: `refresh` and `refresh-now`, with no parameter
// The window toggle stays in extension.js. No call carries text the extension
// or the user wrote, and none carries anything from an agent.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {isStarter} from './starters.js';

const APP_ID = 'io.github.victorytek.VexosAssistant';
const APP_PATH = '/io/github/victorytek/VexosAssistant';
const USAGE_ID = 'io.github.victorytek.VexosAssistant.Usage';
const USAGE_PATH = '/io/github/victorytek/VexosAssistant/Usage';

export class Bus {
    constructor(cancellable) {
        this._cancellable = cancellable;
    }

    // A starter tile: only a known id goes out.
    draft(id) {
        if (!isStarter(id))
            return;
        this._activate(APP_ID, APP_PATH, 'draft', [new GLib.Variant('s', id)]);
    }

    newSession() {
        this._activate(APP_ID, APP_PATH, 'new-session', []);
    }

    terminal() {
        this._activate(APP_ID, APP_PATH, 'terminal', []);
    }

    // Ask the usage service for fresh numbers. `force` skips its 15 s interval
    // and the wait after a refused sign-in; it never skips any other backoff.
    refreshUsage(force) {
        this._activate(USAGE_ID, USAGE_PATH, force ? 'refresh-now' : 'refresh', []);
    }

    _activate(id, path, action, parameter) {
        Gio.DBus.session.call(id, path, 'org.freedesktop.Application',
            'ActivateAction', new GLib.Variant('(sava{sv})', [action, parameter, {}]),
            null, Gio.DBusCallFlags.NONE, -1, this._cancellable,
            (connection, result) => {
                try {
                    connection.call_finish(result);
                } catch (e) {
                    if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                        console.warn(`VexOS Assistant: ${action} failed: ${e.message}`);
                }
            });
    }
}
