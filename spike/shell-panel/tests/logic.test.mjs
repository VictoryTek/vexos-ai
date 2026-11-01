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

test('inputNotice: none, too long, and full (the entry may have cut a paste)', () => {
    assert.equal(L.inputNotice(''), 'none');
    assert.equal(L.inputNotice(undefined), 'none');
    assert.equal(L.inputNotice('hi'), 'none');
    assert.equal(L.inputNotice('a'.repeat(L.MAX_SEND_BYTES)), 'none');
    assert.equal(L.inputNotice('a'.repeat(L.MAX_SEND_BYTES + 1)), 'long');
    assert.equal(L.inputNotice('a'.repeat(65530)), 'long');
    assert.equal(L.inputNotice('a'.repeat(65531)), 'full');
    assert.equal(L.inputNotice('a'.repeat(65534)), 'full');
    // the entry keeps whole characters, so 3 and 4 byte text stops a few bytes short
    assert.equal(L.inputNotice('日'.repeat(21843)), 'long');   // 65529 bytes
    assert.equal(L.inputNotice('日'.repeat(21844)), 'full');   // 65532 bytes
    assert.equal(L.inputNotice('😀'.repeat(16383)), 'full');   // 65532 bytes
    assert.equal(L.inputNotice('😀'.repeat(15000)), 'none');   // exactly 60000 bytes
    assert.equal(L.inputNotice('😀'.repeat(15001)), 'long');
});

test('inputNoticeState: a cut paste stays flagged until the box is emptied', () => {
    const a = n => 'a'.repeat(n);
    const st = L.inputNoticeState;
    assert.deepEqual(st(false, ''), {cut: false, kind: 'none'});
    assert.deepEqual(st(true, ''), {cut: false, kind: 'none'});
    assert.deepEqual(st(false, undefined), {cut: false, kind: 'none'});
    assert.deepEqual(st(false, 'hi'), {cut: false, kind: 'none'});
    assert.deepEqual(st(false, a(61000)), {cut: false, kind: 'long'});
    assert.deepEqual(st(false, a(65531)), {cut: true, kind: 'full'});
    // once cut, it stays: shorter (still too long, or sendable), even whitespace only
    assert.deepEqual(st(true, a(65531)), {cut: true, kind: 'full'});
    assert.deepEqual(st(true, a(61000)), {cut: true, kind: 'cut-long'});
    assert.deepEqual(st(true, a(60000)), {cut: true, kind: 'cut'});
    assert.deepEqual(st(true, 'hi'), {cut: true, kind: 'cut'});
    assert.deepEqual(st(true, ' '), {cut: true, kind: 'cut'});
});

test('inputNoticeState: paste cut, shorten, send, then a new text starts clean', () => {
    const st = L.inputNoticeState;
    let s = st(false, 'a'.repeat(65534));          // a 1 MiB paste, cut by the entry
    assert.equal(s.kind, 'full');
    s = st(s.cut, 'a'.repeat(62000));              // some deleted: still too long
    assert.equal(s.kind, 'cut-long');
    s = st(s.cut, 'a'.repeat(59000));              // shortened: sendable, still warned
    assert.equal(s.kind, 'cut');
    assert.equal(L.inputVerdict('a'.repeat(59000)), 'send');   // sending stays allowed
    s = st(s.cut, '');                             // sent or emptied
    assert.deepEqual(s, {cut: false, kind: 'none'});
    s = st(s.cut, 'hello');                        // a fresh text is not warned
    assert.deepEqual(s, {cut: false, kind: 'none'});
});

test('whatever may have been cut or is over the limit is never sendable', () => {
    for (let n = L.MAX_SEND_BYTES - 2; n <= L.INPUT_LIMIT_BYTES + 5; n += 1) {
        const t = 'a'.repeat(n);
        if (L.inputNotice(t) !== 'none')
            assert.equal(L.inputVerdict(t), 'too-long', String(n));
        else
            assert.equal(L.inputVerdict(t), 'send', String(n));
    }
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
    assert.equal(L.MAX_SEND_BYTES, 60000);
    assert.equal(L.INPUT_LIMIT_BYTES, 65534);
    // a text the entry may have cut (near its limit) must always be too long to send
    assert.ok(L.MAX_SEND_BYTES < L.INPUT_LIMIT_BYTES - 3);
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
    assert.deepEqual(imports, ['./fake.js', './logic.js', 'gi://Clutter', 'gi://Cogl', 'gi://GLib', 'gi://GObject',
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

test('scan: an over-long text is refused with a notice and never cut or sent', () => {
    const c = code(read('extension.js'));
    // the notice is refreshed on every change of the text
    const changed = c.slice(c.indexOf('_inputChanged() {'));
    assert.ok(changed.slice(0, changed.indexOf('\n    }\n')).includes('this._showInputNotice();'));
    // _send returns on 'too-long' before it adds a row
    const send = c.slice(c.indexOf('_send() {'));
    const body = send.slice(0, send.indexOf('\n    }\n'));
    assert.ok(body.indexOf("verdict === 'too-long'") > 0 &&
        body.indexOf("verdict === 'too-long'") < body.indexOf('_addRow'));
    assert.ok(/too-long[^]*?return;[^]*?_addRow/.test(body));
    // nothing shortens the text in the box
    assert.ok(!/ct\.text\.(slice|substring|substr)\(|(ct|clutter_text)\.set_text\(|ct\.text = ct\.text/.test(c));
    // each notice kind is wired to its own fixed text, and the text never claims
    // that nothing was cut (the box may have cut a paste before this text shrank)
    assert.ok(/kind === 'full'\)\s*this\._notice\.text = 'The box is full/.test(c));
    assert.ok(/kind === 'cut-long'\)\s*this\._notice\.text = 'Anything pasted past the 64 KiB limit was cut and is not here\. Still too long/.test(c));
    assert.ok(/kind === 'cut'\)\s*this\._notice\.text = 'Anything pasted past the 64 KiB limit was cut and is not here\. You can still send/.test(c));
    assert.ok(/kind === 'long'\)\s*this\._notice\.text = 'Too long to send/.test(c));
    // the flag is kept from one change of the text to the next, starts false in
    // every new panel, and Send never consults it (sending stays allowed)
    assert.ok(c.includes('this._cut = state.cut;'));
    assert.ok(c.includes('this._cut = false;'));
    const sendBody = c.slice(c.indexOf('_send() {'));
    assert.ok(!sendBody.slice(0, sendBody.indexOf('\n    }\n')).includes('_cut'));
    assert.ok(!/[Nn]othing was cut/.test(c));
});

test('scan: Enter sends only through the text activate signal, which runs after the input method', () => {
    // Clutter binds Enter (no modifier) to `activate` inside the text's own key
    // handling, after the input method has had the key. A key-press handler on
    // the text would see Enter first, before the input method, and could send
    // in the middle of a composition.
    const c = code(read('extension.js'));
    assert.ok(c.includes('activatable: true'));
    assert.ok(!c.includes('activatable: false'));
    assert.ok(c.includes("ct.connect('activate', this._guard(() => this._activate()));"));
    assert.ok(!c.includes("'key-press-event'"));
    assert.ok(!/KEY_Return|KEY_KP_Enter|KEY_ISO_Enter/.test(c));
    // and `_activate` has one caller: the signal handler, after the input method
    assert.equal(c.match(/this\._activate\(\)/g).length, 1);
    // _send has one caller, _activate, which does not send while a composition shows
    assert.equal(c.match(/this\._send\(\)/g).length, 1);
    const fn = c.slice(c.indexOf('_activate() {'));
    const body = fn.slice(0, fn.indexOf('\n    }\n'));
    const guard = body.indexOf('this._entry.clutter_text.has_preedit()');
    assert.ok(guard > 0 && guard < body.indexOf('this._send()'), body);
    assert.ok(/if \(this\._entry\.clutter_text\.has_preedit\(\)\)\s*return;/.test(body), body);
});

test('constants: the selection colours are fixed hex strings', () => {
    assert.equal(L.SELECTION_BG, '#3584e4');
    assert.equal(L.SELECTION_FG, '#ffffff');
});

test('scan: every selectable label gets readable selection colours', () => {
    // St.Label ignores the CSS selection colours, and the default is a white
    // block with light text, so they are set on the text itself.
    const c = code(read('extension.js'));
    const selectable = c.match(/selectable: true/g).length;
    assert.equal(c.match(/selection_color: /g)?.length, selectable);
    assert.equal(c.match(/selected_text_color: /g)?.length, selectable);
    assert.ok(c.includes("import Cogl from 'gi://Cogl';"));
    assert.ok(c.includes('selection_color: Cogl.Color.from_string(Logic.SELECTION_BG)[1]'));
    assert.ok(c.includes('selected_text_color: Cogl.Color.from_string(Logic.SELECTION_FG)[1]'));
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
