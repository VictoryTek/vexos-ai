// Run with: node --test spike/shell-panel/tests/   (SPIKE=<dir> to test a copy)
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, readdirSync, statSync, existsSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const dir = resolve(process.env.SPIKE ??
    join(here, '..', 'vexos-shell-spike@victorytek.github.io'));
const L = await import(pathToFileURL(join(dir, 'logic.js')));
const bytes = s => Buffer.byteLength(s, 'utf8');

test('visible: plain text and tab/newline are unchanged', () => {
    assert.equal(L.visible('héllo wörld\ttab\nnext 日本語 😀'), 'héllo wörld\ttab\nnext 日本語 😀');
});

test('visible: markup characters stay as they are (plain text, not escaped)', () => {
    assert.equal(L.visible('<b>x</b> &amp; "q" %s'), '<b>x</b> &amp; "q" %s');
});

test('visible: C0, DEL and C1 controls are shown as \\u{hex}', () => {
    assert.equal(L.visible('a\u0000b\u0007c\u001bd\u007fe\u0085f\u009fg'),
        'a\\u{0}b\\u{7}c\\u{1b}d\\u{7f}e\\u{85}f\\u{9f}g');
});

test('visible: every bidi control and separator is shown, none passes through', () => {
    const cps = [0x061c, 0x200e, 0x200f, 0x2028, 0x2029,
        0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2066, 0x2067, 0x2068, 0x2069];
    for (const cp of cps) {
        const out = L.visible(`x${String.fromCodePoint(cp)}y`);
        assert.equal(out, `x\\u{${cp.toString(16)}}y`, cp.toString(16));
    }
});

test('visible: neighbours of the escaped ranges are kept', () => {
    for (const cp of [0x1f, 0xa0, 0x061b, 0x061d, 0x200d, 0x2010, 0x202f, 0x2065, 0x206a, 0xe000]) {
        if (cp === 0x1f)
            continue; // 0x1f is a control and is escaped
        assert.equal(L.visible(String.fromCodePoint(cp)), String.fromCodePoint(cp), cp.toString(16));
    }
    assert.equal(L.visible('\u001f'), '\\u{1f}');
    assert.equal(L.visible(' '), ' ');
});

test('visible: CRLF is one newline, a lone CR is shown', () => {
    assert.equal(L.visible('a\r\nb'), 'a\nb');
    assert.equal(L.visible('a\rb'), 'a\\u{d}b');
});

test('visible: lone surrogates are shown, pairs are kept', () => {
    assert.equal(L.visible('a\ud800b'), 'a\\u{d800}b');
    assert.equal(L.visible('a\udfffb'), 'a\\u{dfff}b');
    assert.equal(L.visible('😀'), '😀');
    assert.equal(L.visible('\ude00\ud83d'), '\\u{de00}\\u{d83d}');
});

test('visible: a backslash is doubled only in card style', () => {
    assert.equal(L.visible('a\\u{41}'), 'a\\u{41}');
    assert.equal(L.visible('a\\u{41}', {backslash: true}), 'a\\\\u{41}');
    // a real escape and a literal one differ
    assert.notEqual(L.visible('‮', {backslash: true}), L.visible('\\u{202e}', {backslash: true}));
});

test('visible: not a string gives an empty string', () => {
    for (const v of [undefined, null, 5, {}, []])
        assert.equal(L.visible(v), '');
});

test('capText: short text is returned as is', () => {
    assert.equal(L.capText('hello'), 'hello');
    assert.equal(L.capText(''), '');
});

test('capText: exactly MAX_BYTES is not marked, one more byte is', () => {
    const exact = 'a'.repeat(L.MAX_BYTES);
    assert.equal(L.capText(exact), exact);
    const over = L.capText(exact + 'a');
    assert.ok(over.startsWith(exact + '\n… shortened: 1 units'), over.slice(-60));
});

test('capText: never splits a character; the result stays within the byte cap', () => {
    for (const ch of ['é', '日', '😀']) {
        const out = L.capText(ch.repeat(L.MAX_BYTES));
        const body = out.slice(0, out.indexOf('\n… shortened'));
        assert.ok(bytes(body) <= L.MAX_BYTES, ch);
        assert.ok(bytes(body) > L.MAX_BYTES - 4, ch);
        assert.equal(body, ch.repeat([...body].length), ch);
        assert.ok(!/[\ud800-\udfff]/.test(body.replace(/[\ud800-\udbff][\udc00-\udfff]/g, '')), ch);
    }
});

test('capText: lines are capped at MAX_LINES', () => {
    const exact = Array.from({length: L.MAX_LINES}, (_, i) => `l${i}`).join('\n');
    assert.equal(L.capText(exact), exact);
    const out = L.capText(`${exact}\nextra\nmore`);
    const body = out.slice(0, out.indexOf('\n… shortened'));
    assert.equal(body.split('\n').length, L.MAX_LINES);
    assert.ok(out.includes('… shortened: 11 units'), out.slice(-50));
});

test('capText: the cap applies after escaping (an escape never exceeds it)', () => {
    const out = L.capText('‮'.repeat(L.MAX_BYTES));
    const body = out.slice(0, out.indexOf('\n… shortened'));
    assert.ok(bytes(body) <= L.MAX_BYTES);
    assert.ok(!body.includes('‮'));
    assert.ok(body.startsWith('\\u{202e}\\u{202e}'));
});

test('capText: huge input is cheap and says what was dropped', () => {
    const t0 = performance.now();
    const out = L.capText('x'.repeat(5 * 1024 * 1024));
    assert.ok(performance.now() - t0 < 500, 'under half a second');
    assert.ok(bytes(out) < L.MAX_BYTES + 100);
    const m = /shortened: (\d+) units/.exec(out);
    assert.equal(Number(m[1]), 5 * 1024 * 1024 - L.MAX_BYTES);
});

test('capText: only the first MAX_RAW units are read (the rest is counted, not escaped)', () => {
    const out = L.capText('y'.repeat(L.MAX_RAW) + '\u202e'.repeat(1000));
    assert.ok(!out.includes('202e'));
    const hidden = Number(/shortened: (\d+) units/.exec(out)[1]);
    assert.equal(hidden, 1000 + (L.MAX_RAW - L.MAX_BYTES));
});

test('capText: control characters never reach the output', () => {
    const out = L.capText('a\u0000b\u001b[31m‮\ud800');
    assert.ok(!/[\u0000-\u0008\u000b-\u001f\u007f-\u009f‪-‮\ud800-\udfff]/.test(out));
});

test('capRows: keeps the newest MAX_ROWS, in order', () => {
    const rows = Array.from({length: L.MAX_ROWS + 5}, (_, i) => i);
    const out = L.capRows(rows);
    assert.equal(out.length, L.MAX_ROWS);
    assert.equal(out[0], 5);
    assert.equal(out.at(-1), L.MAX_ROWS + 4);
    assert.deepEqual(L.capRows([1, 2]), [1, 2]);
    assert.equal(L.capRows(rows.slice(0, L.MAX_ROWS)).length, L.MAX_ROWS);
    assert.notEqual(L.capRows([1]), [1]);
});

test('inputVerdict: empty, send, too-long, and the exact boundary', () => {
    assert.equal(L.inputVerdict(''), 'empty');
    assert.equal(L.inputVerdict(' \n\t '), 'empty');
    assert.equal(L.inputVerdict(undefined), 'empty');
    assert.equal(L.inputVerdict('hi'), 'send');
    assert.equal(L.inputVerdict('a'.repeat(L.MAX_SEND_BYTES)), 'send');
    assert.equal(L.inputVerdict('a'.repeat(L.MAX_SEND_BYTES + 1)), 'too-long');
    // multi-byte: 3 bytes each
    assert.equal(L.inputVerdict('日'.repeat(L.MAX_SEND_BYTES / 3)), 'send');
    assert.equal(L.inputVerdict('日'.repeat(L.MAX_SEND_BYTES / 3 + 1)), 'too-long');
});

test('utf8Length matches Buffer.byteLength', () => {
    for (const s of ['', 'a', 'é', '日', '😀', 'a😀日é\n'])
        assert.equal(L.utf8Length(s), bytes(s), s);
});

test('panelRect: laptop 1280x800 with a 32 px bar', () => {
    const r = L.panelRect({x: 0, y: 0, width: 1280, height: 800}, 32);
    assert.deepEqual(r, {x: 280, width: 720, height: 520, hiddenY: -488, targetY: 32});
});

test('panelRect: 1080p, ultrawide, tall and second monitor', () => {
    assert.deepEqual(L.panelRect({x: 0, y: 0, width: 1920, height: 1080}, 32),
        {x: 480, width: 960, height: 629, hiddenY: -597, targetY: 32});
    assert.equal(L.panelRect({x: 0, y: 0, width: 3440, height: 1440}, 32).width, 960);
    assert.equal(L.panelRect({x: 0, y: 0, width: 1080, height: 1920}, 32).height, 1000);
    const s = L.panelRect({x: 1920, y: 0, width: 2560, height: 1440}, 0);
    assert.equal(s.x, 1920 + 800);
    assert.equal(s.targetY, 0);
    assert.equal(s.hiddenY, -s.height);
});

test('panelRect: always inside the monitor, hidden entirely above the bar', () => {
    for (const w of [1, 300, 719, 720, 1400, 1920, 7680])
        for (const h of [1, 300, 600, 1080, 4320]) {
            const m = {x: 10, y: 20, width: w, height: h};
            const r = L.panelRect(m, 32);
            assert.ok(r.x >= m.x && r.x + r.width <= m.x + m.width, JSON.stringify([m, r]));
            assert.ok(r.height <= Math.max(0, h - 32));
            assert.equal(r.hiddenY + r.height, r.targetY);
            assert.equal(r.targetY, m.y + 32);
        }
});

test('panelRect: bad numbers give finite values', () => {
    const r = L.panelRect({x: NaN, y: undefined, width: Infinity, height: -5}, NaN);
    for (const v of Object.values(r))
        assert.ok(Number.isFinite(v), JSON.stringify(r));
});

test('scrollToShow: moves as little as possible', () => {
    assert.equal(L.scrollToShow(100, 50, 120, 140), 100);   // already visible
    assert.equal(L.scrollToShow(100, 50, 90, 110), 90);     // above: align top
    assert.equal(L.scrollToShow(100, 50, 140, 170), 120);   // below: align bottom
    assert.equal(L.scrollToShow(100, 50, 100, 150), 100);   // exactly fits
    assert.equal(L.scrollToShow(100, 50, 99, 150), 99);
    assert.equal(L.scrollToShow(100, 50, 101, 151), 101);
});

test('scrollToShow: bad numbers leave the value alone', () => {
    assert.equal(L.scrollToShow(100, NaN, 10, 20), 100);
    assert.equal(L.scrollToShow(100, 50, undefined, 20), 100);
    assert.equal(L.scrollToShow(NaN, 50, 10, 20), 0);
});

test('inputHeight: between one line and the cap', () => {
    assert.equal(L.inputHeight(10, 30, 180), 30);
    assert.equal(L.inputHeight(100, 30, 180), 100);
    assert.equal(L.inputHeight(500, 30, 180), 180);
    assert.equal(L.inputHeight(30, 30, 180), 30);
    assert.equal(L.inputHeight(180, 30, 180), 180);
    assert.equal(L.inputHeight(NaN, 30, 180), 30);
    assert.equal(L.inputHeight(100, 50, 20), 50);
});

test('constants: the stated caps and slide', () => {
    assert.equal(L.SLIDE_MS, 250);
    assert.equal(L.MAX_BYTES, 16 * 1024);
    // the text plus the mark must fit Clutter's text buffer (65535 bytes)
    assert.ok(L.MAX_BYTES + 100 < 65535);
    assert.equal(L.MAX_LINES, 2000);
    assert.equal(L.MAX_ROWS, 200);
    assert.equal(L.MAX_SEND_BYTES, 72 * 1024);
    assert.equal(L.INPUT_MAX_LINES, 8);
});

// Source scans: what the extension may not contain.
const read = f => readFileSync(join(dir, f), 'utf8');
const code = text => text.split('\n').filter(l => !l.trim().startsWith('//')).join('\n');
const files = ['extension.js', 'logic.js', 'fake.js'];

test('scan: no markup, eval, process, file, network, bus or timer APIs', () => {
    for (const f of files) {
        const c = code(read(f));
        for (const word of ['markup', 'use_markup', 'set_markup', 'eval(', 'Function(', 'import(',
            'Gio', 'Subprocess', 'spawn', 'DBus', 'Soup', 'GFile', 'file_get_contents',
            'timeout_add', 'idle_add', 'later_add', 'get_laters', 'GLib.Source', 'setTimeout', 'setInterval', 'console.log'])
            assert.ok(!c.includes(word), `${f}: ${word}`);
    }
});

test('scan: logic.js and fake.js import nothing', () => {
    for (const f of ['logic.js', 'fake.js'])
        assert.ok(!/^\s*import\b/m.test(code(read(f))), f);
});

test('scan: only known imports in extension.js', () => {
    const imports = [...read('extension.js').matchAll(/^import .* from '([^']+)';$/gm)].map(m => m[1]).sort();
    assert.deepEqual(imports, ['./fake.js', './logic.js', 'gi://Clutter', 'gi://GLib', 'gi://GObject',
        'gi://Meta', 'gi://Pango', 'gi://Shell', 'gi://St',
        'resource:///org/gnome/shell/extensions/extension.js',
        'resource:///org/gnome/shell/ui/main.js'].sort());
});

test('scan: the only stage signal is after-paint, connected and disconnected in one place', () => {
    const c = code(read('extension.js'));
    assert.equal(c.match(/global\.stage\.connect\(/g).length, 1);
    assert.ok(c.includes("global.stage.connect('after-paint'"));
    assert.equal(c.match(/global\.stage\.disconnect\(/g).length, 1);
    assert.ok(!/global\.(display|window_manager|compositor)\.connect/.test(c));
    assert.ok(!/Main\.\w+\.connect\(/.test(c));
});

test('scan: the clipboard is the one deliberate API and only writes', () => {
    const c = code(read('extension.js'));
    assert.equal(c.match(/Clipboard/g).length, 2);
    assert.ok(c.includes('set_text(St.ClipboardType.CLIPBOARD, shown)'));
    assert.ok(!c.includes('get_text'));
});

test('scan: every handler goes through _guard, and teardown pops the grab itself', () => {
    const c = code(read('extension.js'));
    // each connect( on an actor wraps its callback in _guard, except the stage
    // one (also guarded) and the panel's key vfunc (guarded at construction)
    const connects = c.match(/\.connect\(/g).length;
    const guarded = c.match(/\.connect\([^;]*?this\._guard\(/gs).length;
    assert.equal(connects, guarded);
    assert.ok(c.includes('new Panel(this._guard('));
    const td = c.slice(c.indexOf('_teardown() {'), c.indexOf('_build(rect) {'));
    assert.ok(td.includes('Main.popModal(grab)'));
    assert.ok(td.indexOf('Main.popModal(grab)') < td.indexOf('panel.destroy()'));
    // the ease callbacks may not be the only thing that pops
    assert.equal(c.match(/Main\.popModal\(/g).length, 1);
});

test('scan: enable() adds the one keybinding; disable() removes it and tears the panel down', () => {
    const c = code(read('extension.js'));
    assert.equal(c.match(/addKeybinding\(/g).length, 1);
    assert.equal(c.match(/removeKeybinding\(/g).length, 1);
    const dis = c.slice(c.indexOf('    disable() {'), c.indexOf('    // Wraps a handler'));
    assert.ok(dis.includes('Main.wm.removeKeybinding(SHORTCUT)'));
    assert.ok(dis.includes('this._teardown()'));
    const en = c.slice(c.indexOf('    enable() {'), c.indexOf('    disable() {'));
    assert.ok(en.includes('Main.wm.addKeybinding(SHORTCUT'));
    // the keybinding is the only thing enable() registers
    assert.ok(!/\.connect\(|timeout_add|idle_add/.test(en));
});

test('scan: the log line carries an error name, never a message or text', () => {
    const c = code(read('extension.js'));
    assert.equal(c.match(/console\.\w+\(/g).length, 1);
    assert.ok(c.includes('handler error: ${e?.name ?? \'Error\'}'));
    assert.ok(!/\.message|\.stack/.test(c));
});

test('scan: the only widget text comes from capText, visible or fixed strings', () => {
    const c = code(read('extension.js'));
    for (const m of c.matchAll(/(?<![\w])text: ([^,}\n]+)/g)) {
        const v = m[1].trim();
        assert.ok(/^'[^']*'$/.test(v) || v.startsWith('Logic.visible(') || v === 'shown' ||
            v.startsWith("role === 'you'") || v === 'ct.text', v); // ct.text goes to _addRow, which caps it
    }
    assert.ok(!/\.text = (?!'|result|''|ct\.text)/.test(c.replace(/this\._notice\.text = '[^']*'/g, '')),
        'only fixed strings are assigned to .text');
});

test('scan: nothing outside the spike names its uuid (never packaged or enabled)', () => {
    const root = resolve(here, '..', '..', '..');
    const skip = new Set(['target', 'result', '.git', 'node_modules']);
    const hits = [];
    const walk = d => {
        for (const name of readdirSync(d)) {
            if (skip.has(name))
                continue;
            const p = join(d, name);
            if (p === join(root, 'spike', 'shell-panel') || p === join(root, '.claude') ||
                p === join(root, '.github', 'docs'))
                continue;
            if (statSync(p).isDirectory())
                walk(p);
            else if (statSync(p).size < 2_000_000 &&
                readFileSync(p, 'latin1').includes('vexos-shell-spike'))
                hits.push(p);
        }
    };
    if (existsSync(join(root, 'flake.nix')))
        walk(root);
    assert.deepEqual(hits, []);
});

test('metadata: its own uuid and schema, shell 50 only', () => {
    const m = JSON.parse(read('metadata.json'));
    assert.equal(m.uuid, 'vexos-shell-spike@victorytek.github.io');
    assert.deepEqual(m['shell-version'], ['50']);
    assert.equal(m['settings-schema'], 'org.gnome.shell.extensions.vexos-shell-spike');
    assert.ok(!JSON.stringify(m).includes('vexos-assistant'));
});

test('fake data: has the required hard cases', async () => {
    const {CONVERSATION, CARD} = await import(pathToFileURL(join(dir, 'fake.js')));
    const all = CONVERSATION.map(r => r.text);
    assert.ok(all.some(t => t.length > 150_000), 'a very long message');
    assert.ok(CONVERSATION.some(r => r.kind === 'code' && r.text.split('\n').length >= 400), 'a long code block');
    assert.ok(CONVERSATION.some(r => r.kind === 'code' && r.text.length > 2500 && !r.text.includes('\n')), 'a wide line');
    assert.ok(all.some(t => t.includes('<b>') && t.includes('&amp;')), 'markup characters');
    assert.ok(all.some(t => t.includes('‮') && t.includes('\u0000') && t.includes('\ud800')), 'control characters');
    assert.ok(CARD.command.includes('‮'));
    // and the cap tames all of them
    for (const t of all) {
        const out = L.capText(t);
        assert.ok(bytes(out) <= L.MAX_BYTES + 100);
        assert.ok(!/[\u0000-\u0008\u000b-\u001f\u007f-\u009f‪-‮⁦-⁩\ud800-\udfff]/.test(out));
    }
});
