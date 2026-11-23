// Run with: node --test tests/extension/layout.test.mjs   (EXT=<dir> to test a built copy)
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, readdirSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const dir = resolve(process.env.EXT ??
    'extension/vexos-assistant@victorytek.github.io');
const layout = await import(pathToFileURL(join(dir, 'layout.js')));
const {parseMode, dockedRect, floatingRect, normalSize, DEFAULT_NORMAL,
    slideStart, slideDecision, SLIDE_MS, SLIDE_EASING, FALLBACK_MS} = layout;

test('parseMode: both strings', () => {
    assert.equal(parseMode('floating'), 'floating');
    assert.equal(parseMode('docked'), 'docked');
});

test('parseMode: anything else is docked', () => {
    const bad = ['', 'bogus', ' floating', 'floating ', 'FLOATING', 'floating\n',
        'floating\0', '\u001b[31mfloating', 'docked\u0007', 'floating'.repeat(200000),
        undefined, null, 0, 1, {}, [], ['floating'], true];
    for (const v of bad)
        assert.equal(parseMode(v), 'docked', String(v).slice(0, 20));
});

const within = (r, a) => r.x >= a.x && r.y >= a.y &&
    r.x + r.width <= a.x + a.width && r.y + r.height <= a.y + a.height;

test('dockedRect: laptop 1280x768 work area', () => {
    const a = {x: 0, y: 32, width: 1280, height: 768};
    assert.deepEqual(dockedRect(a), {x: 280, y: 32, width: 720, height: 520});
});

test('dockedRect: 1080p', () => {
    const a = {x: 0, y: 32, width: 1920, height: 1048};
    assert.deepEqual(dockedRect(a), {x: 480, y: 32, width: 960, height: 629});
});

test('dockedRect: tall portrait screen is capped at the maximum height', () => {
    const a = {x: 0, y: 32, width: 1080, height: 1888};
    const r = dockedRect(a);
    assert.equal(r.height, 1000);
    assert.equal(r.width, 720);
    assert.ok(within(r, a));
});

test('dockedRect: ultrawide stays at the maximum width, centred', () => {
    const a = {x: 0, y: 32, width: 3440, height: 1408};
    assert.deepEqual(dockedRect(a), {x: 1240, y: 32, width: 960, height: 845});
});

test('dockedRect: a work area smaller than the minimum is filled, not exceeded', () => {
    const a = {x: 0, y: 32, width: 600, height: 400};
    assert.deepEqual(dockedRect(a), {x: 0, y: 32, width: 600, height: 400});
    const z = dockedRect({x: 5, y: 7, width: 0, height: 0});
    assert.deepEqual(z, {x: 5, y: 7, width: 0, height: 0});
});

test('dockedRect: a second monitor keeps its own origin', () => {
    const a = {x: 1920, y: 0, width: 2560, height: 1440};
    const r = dockedRect(a);
    assert.deepEqual(r, {x: 1920 + 800, y: 0, width: 960, height: 864});
    assert.ok(within(r, a));
});

test('dockedRect: always inside the work area', () => {
    for (const w of [1, 300, 719, 720, 721, 1400, 1920, 7680])
        for (const h of [1, 300, 519, 520, 867, 1000, 1700, 4320]) {
            const a = {x: 10, y: 20, width: w, height: h};
            const r = dockedRect(a);
            assert.ok(within(r, a), JSON.stringify([a, r]));
            assert.equal(r.y, a.y);
        }
});

test('floatingRect: the normal size, centred and capped', () => {
    const a = {x: 0, y: 32, width: 1920, height: 1048};
    assert.deepEqual(floatingRect(a, {width: 760, height: 820}), {x: 580, y: 146, width: 760, height: 820});
    assert.deepEqual(floatingRect({x: 0, y: 0, width: 500, height: 400}, {width: 760, height: 820}),
        {x: 0, y: 0, width: 500, height: 400});
});

test('normalSize: the first-show size, unless it is already the docked size', () => {
    const a = {x: 0, y: 32, width: 1920, height: 1048};
    assert.deepEqual(normalSize({width: 800, height: 600}, a), {width: 800, height: 600});
    const d = dockedRect(a);
    assert.deepEqual(normalSize({width: d.width, height: d.height}, a), DEFAULT_NORMAL);
});

const sources = ['extension.js', 'layout.js'].map(f => [f, readFileSync(join(dir, f), 'utf8')]);
const code = text => text.split('\n').filter(l => !l.trim().startsWith('//')).join('\n');

// Every module of the extension, by name.
const modules = ['extension.js', 'layout.js', 'indicator.js', 'usage_view.js', 'usage_store.js',
    'bus.js', 'starters.js', 'tooltip.js'];
const all = Object.fromEntries(modules.map(f => [f, code(readFileSync(join(dir, f), 'utf8'))]));

test('the extension is exactly these modules', () => {
    const present = readdirSync(dir).filter(f => f.endsWith('.js')).sort();
    assert.deepEqual(present, [...modules].sort());
});

test('source scan: the extension carries no agent text, starts nothing and runs no code', () => {
    for (const [name, text] of Object.entries(all)) {
        for (const word of ['get_title', 'Subprocess', 'spawn', 'Clipboard', 'Notification',
            'Eval', 'GLib.file_get_contents', 'read_bytes', 'eval(', 'Function(', 'GLib.spawn'])
            assert.ok(!text.includes(word), `${name}: ${word}`);
    }
});

test('source scan: the only process the extension starts is VexPortal on its AI page, from a fixed command', () => {
    for (const [name, text] of Object.entries(all))
        assert.equal([...text.matchAll(/create_from_commandline\(/g)].length, name === 'indicator.js' ? 1 : 0, name);
    assert.ok(all['indicator.js'].includes("create_from_commandline('vexportal --page ai', 'VexPortal',"));
});

test('source scan: only usage_store.js touches the file system, and it only reads one file', () => {
    for (const [name, text] of Object.entries(all)) {
        if (name === 'usage_store.js')
            continue;
        for (const word of ['Gio.File', 'load_contents', 'replace_contents', 'FileMonitor',
            'monitor_file', 'file_set_contents', 'GLib.mkdir', 'GLib.unlink', 'GLib.rename',
            'GLib.get_user_state_dir'])
            assert.ok(!text.includes(word), `${name}: ${word}`);
    }
    const store = all['usage_store.js'];
    for (const word of ['replace', 'create', 'delete', 'append_to', 'copy', 'move', 'trash', 'set_contents'])
        assert.ok(!new RegExp(`\\b\\w*\\.${word}\\w*\\(`).test(store), `usage_store.js: ${word}`);
    assert.deepEqual([...store.matchAll(/new_for_path\(/g)].length, 1);
    assert.ok(store.includes("'vexos-ai', 'usage.json'"));
});

test('source scan: the window toggle is the only D-Bus call in extension.js, with no parameter', () => {
    const ext = code(sources[0][1]);
    assert.equal(ext.match(/\.call\(/g).length, 1);
    assert.ok(ext.includes("'ActivateAction', new GLib.Variant('(sava{sv})', ['toggle', [], {}])"));
});

test('source scan: bus.js makes one kind of call, and only these actions', () => {
    const bus = all['bus.js'];
    // One call site: ActivateAction on an app id.
    assert.equal(bus.match(/\.call\(/g).length, 1);
    assert.ok(bus.includes("'org.freedesktop.Application',\n            'ActivateAction'"));
    assert.ok(bus.includes("new GLib.Variant('(sava{sv})', [action, parameter, {}])"));
    // The actions, their targets and their parameters.
    const used = [...bus.matchAll(/this\._activate\((\w+), (\w+), ([^,]+), (\[[^\]]*\]|\[new GLib\.Variant\('s', id\)\])\)/g)]
        .map(m => `${m[1]} ${m[3].trim()} ${m[4]}`).sort();
    assert.deepEqual(used, [
        "APP_ID 'draft' [new GLib.Variant('s', id)]",
        "APP_ID 'new-session' []",
        "APP_ID 'prewarm' []",
        "APP_ID 'terminal' []",
        "USAGE_ID force ? 'refresh-now' : 'refresh' []",
    ].sort());
    // A draft goes out only for a known starter id.
    assert.ok(/draft\(id\) \{\s+if \(!isStarter\(id\)\)\s+return;/.test(bus));
    // The two target app ids.
    assert.deepEqual([...bus.matchAll(/const (\w+_ID) = '([^']+)'/g)].map(m => `${m[1]}=${m[2]}`), [
        'APP_ID=io.github.victorytek.VexosAssistant',
        'USAGE_ID=io.github.victorytek.VexosAssistant.Usage',
    ]);
});

test('source scan: no other module calls D-Bus', () => {
    for (const [name, text] of Object.entries(all)) {
        if (name === 'extension.js' || name === 'bus.js')
            continue;
        assert.ok(!text.includes('Gio.DBus') && !/\.call\(/.test(text) && !text.includes('call_sync'),
            name);
    }
});

test('source scan: the known settings keys, and the one setting the extension writes', () => {
    const ext = all['extension.js'];
    assert.ok(!/\.set_\w+\(/.test(ext), 'extension.js never writes a setting');
    assert.deepEqual([...ext.matchAll(/(?:SHORTCUT|MODE_KEY) = '([^']+)'/g)].map(m => m[1]).sort(),
        ['toggle-assistant', 'window-mode']);
    // The panel's consent switch is written in indicator.js and nowhere else, as
    // a boolean from the user's own click.
    for (const [name, text] of Object.entries(all)) {
        const sets = [...text.matchAll(/\.set_\w+\(([^)]*)\)/g)].map(m => m[0]);
        if (name !== 'indicator.js') {
            assert.deepEqual(sets.filter(s => /set_(boolean|string|strv|int|value)/.test(s)), [], name);
            continue;
        }
        const writes = sets.filter(s => /set_(boolean|string|strv|int|value)\(/.test(s));
        assert.equal(writes.length, 2, writes.join('\n'));
        for (const w of writes)
            assert.ok(/^\.set_boolean\(USAGE_KEY, (true|false)\)$/.test(w), w);
    }
    assert.ok(all['indicator.js'].includes("const USAGE_KEY = 'usage-enabled';"));
});

test('source scan: the pure modules import nothing', () => {
    for (const name of ['layout.js', 'usage_view.js', 'starters.js'])
        assert.ok(!/^\s*import\b/m.test(all[name]), name);
});

test('source scan: the top-bar button is added once and gives right and middle click their jobs', () => {
    const ind = all['indicator.js'];
    // Shell 50 opens the menu from a click gesture: left stays the menu's, and
    // right and middle are gestures of our own.
    assert.ok(ind.includes('this._clickGesture?.set_required_button(Clutter.BUTTON_PRIMARY);'));
    assert.ok(!ind.includes('vfunc_event'));
    assert.ok(/_addGesture\(Clutter\.BUTTON_SECONDARY, \(\) => \{\s+this\.menu\.close\(\);\s+this\._onToggleWindow\(\);/.test(ind));
    assert.ok(/_addGesture\(Clutter\.BUTTON_MIDDLE, \(\) => \{\s+if \(this\._enabled\(\)\)\s+this\._bus\.refreshUsage\(true\);/.test(ind));
    assert.ok(ind.includes('new Clutter.ClickGesture({required_button: button})'));
    assert.equal(all['extension.js'].match(/addToStatusArea\(/g).length, 1);
});

test('source scan: the window is hidden from the window list (no dock icon) and shown again on disable', () => {
    const ext = all['extension.js'];
    // Once, in _track, inside a try; the Shell lists an app as running only while
    // it has a window that is not skip-taskbar.
    assert.equal(ext.match(/hide_from_window_list\(/g).length, 1);
    assert.ok(/_track\(window, place\) \{[\s\S]*?try \{\s+window\.hide_from_window_list\(\);\s+\} catch \(e\) \{/.test(ext));
    // Undone only when the extension lets go of the window (restore), in its own try.
    assert.equal(ext.match(/show_in_window_list\(/g).length, 1);
    assert.ok(/if \(restore\) \{\s+try \{\s+window\.show_in_window_list\(\);\s+\} catch \{\}/.test(ext));
});

test('source scan: a window is hidden the moment its app id arrives, not only when first shown', () => {
    const ext = all['extension.js'];
    assert.ok(ext.includes("window.connect('notify::gtk-application-id', identified)"));
    assert.ok(ext.includes("window.connect('notify::wm-class', identified)"));
    assert.ok(/const identified = \(\) => \{\s+if \(!this\._isAssistant\(window\)\)\s+return;\s+this\._unwatch\(window\);\s+this\._track\(window, true\);/.test(ext));
});

test('slide: about 250 ms, ease-out, and a fallback that outlasts the slide start', () => {
    assert.equal(SLIDE_MS, 250);
    assert.equal(SLIDE_EASING, 'EASE_OUT_QUAD');
    assert.ok(FALLBACK_MS >= 300 && FALLBACK_MS <= 1000);
});

test('slideStart: the whole window above the screen edge', () => {
    // Work area under a 32 px top bar: the window must clear the bar too.
    assert.equal(slideStart(520, 32, 0), -552);
    // No bar (second monitor): just its height.
    assert.equal(slideStart(629, 0, 0), -629);
    assert.equal(slideStart(520, 1112, 1080), -552);
    // Always above the docked top edge by at least the window's height.
    for (const h of [0, 1, 400, 1000])
        for (const bar of [0, 24, 32, 64])
            assert.ok(slideStart(h, bar, 0) <= -(h + bar));
});

test('slideStart: bad numbers never move the window below its place', () => {
    for (const [h, y, sy] of [[NaN, 32, 0], [Infinity, 0, 0], [-5, 32, 0], [520, NaN, 0],
        [520, 32, undefined], [undefined, undefined, undefined]])
        assert.ok(slideStart(h, y, sy) <= 0 && Number.isFinite(slideStart(h, y, sy)),
            String([h, y, sy]));
});

test('slideDecision: slide only after the move landed; never invisible for long', () => {
    assert.equal(slideDecision({}), 'wait');
    assert.equal(slideDecision({landed: true}), 'slide');
    assert.equal(slideDecision({timedOut: true}), 'show');
    assert.equal(slideDecision({landed: true, timedOut: true}), 'slide');
    assert.equal(slideDecision({gone: true}), 'abort');
    assert.equal(slideDecision({gone: true, landed: true, timedOut: true}), 'abort');
});

test('source scan: the window is hidden only while parked, and always revealed', () => {
    const ext = code(sources[0][1]);
    // Opacity goes to 0 in one place (the park); everything else restores it.
    assert.equal(ext.match(/opacity = 0;/g).length, 1);
    assert.ok(ext.match(/opacity = 255;/g).length >= 3);
    const place = ext.slice(ext.indexOf('_place(window, entry) {'), ext.indexOf('_actor(window) {'));
    // Floating returns before anything is hidden or deferred.
    assert.ok(/if \(this\._mode\(\) !== DOCKED\)\s+return;/.test(place));
    assert.ok(place.indexOf('this._mode() !== DOCKED') < place.indexOf('this._park('));
    // With the overview open Shell's effect cannot be ended: no park.
    assert.ok(/Main\.overview\.visible\) \{[^}]*this\._apply\(window, entry, DOCKED\)/.test(place));
    // The park waits behind Shell's async map effect, and a failure reveals.
    assert.ok(place.includes('Promise.resolve().then(() => this._park(window, entry))'));
    assert.ok(place.includes('this._reveal(this._actor(window))'));
    const park = ext.slice(ext.indexOf('_park(window, entry) {'), ext.indexOf('_armSlide(window, entry, actor, start) {'));
    assert.ok(park.includes('this._windows?.get(window) !== entry'), 'a closed window is skipped');
    assert.ok(park.indexOf('remove_all_transitions()') < park.indexOf('opacity = 0;'));
    assert.ok(park.includes('this._armSlide(window, entry, actor, start);'));
    assert.ok(park.indexOf('this._armSlide(') < park.indexOf('move_resize_frame'),
        'the landing signals are connected before the move');
    assert.ok(ext.includes('GLib.timeout_add(GLib.PRIORITY_DEFAULT, FALLBACK_MS'));
    assert.ok(ext.includes('GLib.source_remove(slide.timer)'));
    // The reveal parks the actor again after ending transitions, then eases.
    const reveal = ext.slice(ext.indexOf('_reveal(actor, start = null) {'), ext.indexOf('_sync() {'));
    assert.ok(reveal.indexOf('remove_all_transitions()') < reveal.indexOf('translation_y = start;'));
    assert.ok(reveal.indexOf('translation_y = start;') < reveal.indexOf('actor.ease('));
    assert.equal(ext.match(/move_resize_frame\(/g).length, 3);
});

test('source scan: the extension never reads the frame position during the slide', () => {
    const ext = code(sources[0][1]);
    const armed = ext.slice(ext.indexOf('_armSlide(window, entry, actor, start) {'), ext.indexOf('_sync() {'));
    for (const word of ['get_frame_rect', 'get_buffer_rect', 'translation_y;', 'get_position'])
        assert.ok(!armed.includes(word), word);
});

test('source scan: the popup has a fixed width, every button tracks hover, and hints are fixed strings', () => {
    const css = readFileSync(join(dir, 'stylesheet.css'), 'utf8');
    assert.ok(/\.vexos-popup\s*\{[^}]*\bwidth:\s*\d+px/.test(css), 'the popup needs a width, not a min-width');
    assert.ok(!/\.vexos-popup\s*\{[^}]*min-width/.test(css));
    for (const name of ['indicator.js']) {
        const text = all[name];
        // Every St.Button is created with track_hover, or its :hover style never applies.
        const buttons = [...text.matchAll(/new St\.Button\(\{([^}]*)\}\)/g)];
        assert.ok(buttons.length >= 3, 'buttons found');
        for (const b of buttons)
            assert.ok(b[1].includes('track_hover: true'), b[1]);
    }
    // The hover hint reads and sends nothing: only the caller's text, shown.
    const tip = all['tooltip.js'];
    assert.ok(!/Gio\.|\.call\(|get_title|clipboard/i.test(tip));
});

test('source scan: plan limits are on by default and the panel has no opt-in button', () => {
    assert.ok(!all['indicator.js'].includes('Show plan limits'));
    const xml = readFileSync(join(dir, 'schemas', 'org.gnome.shell.extensions.vexos-assistant.gschema.xml'), 'utf8');
    assert.ok(/<key name="usage-enabled" type="b">\s*<default>true<\/default>/.test(xml));
});
