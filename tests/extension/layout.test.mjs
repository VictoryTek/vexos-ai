// Run with: node --test tests/extension/layout.test.mjs   (EXT=<dir> to test a built copy)
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const dir = resolve(process.env.EXT ??
    'extension/vexos-assistant@victorytek.github.io');
const layout = await import(pathToFileURL(join(dir, 'layout.js')));
const {parseMode, dockedRect, floatingRect, normalSize, DEFAULT_NORMAL} = layout;

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
