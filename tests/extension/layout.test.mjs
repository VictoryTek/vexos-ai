// Run with: node --test tests/extension/layout.test.mjs   (EXT=<dir> to test a built copy)
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
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

test('source scan: the extension carries no agent text', () => {
    for (const [name, text] of sources) {
        for (const word of ['get_title', 'Subprocess', 'spawn', 'Gio.File', 'Clipboard',
            'Notification', 'Eval', 'GLib.file_get_contents', 'read_bytes'])
            assert.ok(!code(text).includes(word), `${name}: ${word}`);
    }
});

test('source scan: one D-Bus call, to the app toggle action, with no parameter', () => {
    const ext = code(sources[0][1]);
    assert.equal(ext.match(/\.call\(/g).length, 1);
    assert.ok(ext.includes("'ActivateAction', new GLib.Variant('(sava{sv})', ['toggle', [], {}])"));
});

test('source scan: only the two known settings keys are used, and set is never called', () => {
    const ext = code(sources[0][1]);
    assert.ok(!/\.set_\w+\(/.test(ext), 'the extension never writes a setting');
    assert.deepEqual([...ext.matchAll(/(?:SHORTCUT|MODE_KEY) = '([^']+)'/g)].map(m => m[1]).sort(),
        ['toggle-assistant', 'window-mode']);
});

test('source scan: layout.js imports nothing', () => {
    assert.ok(!/^\s*import\b/m.test(code(sources[1][1])));
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
