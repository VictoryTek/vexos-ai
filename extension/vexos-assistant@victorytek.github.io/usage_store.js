// Reads the usage service's record: one file, read only, never written. The
// only file in the extension that touches the file system.
//
// $XDG_STATE_HOME/vexos-ai/usage.json holds numbers and fixed words (see
// usage_view.js). It is read when it changes, when the panel opens and when the
// refresh timer fires; a failed or oversize read means "no data".

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {MAX_FILE_BYTES, parseRecord} from './usage_view.js';

export class UsageStore {
    constructor(onChange) {
        this._onChange = onChange;
        this._cancellable = new Gio.Cancellable();
        this._record = null;
        this._file = Gio.File.new_for_path(
            GLib.build_filenamev([GLib.get_user_state_dir(), 'vexos-ai', 'usage.json']));
        this._monitor = null;
        this._monitorId = 0;
        try {
            this._monitor = this._file.monitor_file(Gio.FileMonitorFlags.WATCH_MOVES, null);
            this._monitorId = this._monitor.connect('changed', () => this.reload());
        } catch (e) {
            console.warn(`VexOS Assistant: cannot watch the usage file: ${e.message}`);
        }
        this.reload();
    }

    get record() {
        return this._record;
    }

    reload() {
        this._file.load_contents_async(this._cancellable, (file, result) => {
            let record = null;
            try {
                const [ok, bytes] = file.load_contents_finish(result);
                if (ok && bytes.length <= MAX_FILE_BYTES)
                    record = parseRecord(new TextDecoder().decode(bytes));
            } catch (e) {
                if (e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    return;
            }
            this._record = record;
            this._onChange();
        });
    }

    destroy() {
        this._cancellable.cancel();
        if (this._monitor && this._monitorId)
            this._monitor.disconnect(this._monitorId);
        this._monitor?.cancel();
        this._monitor = null;
        this._monitorId = 0;
        this._onChange = () => {};
    }
}
