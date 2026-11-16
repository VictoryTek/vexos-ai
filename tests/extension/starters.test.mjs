// The starter tiles in the extension and in the app are the same list.
// STARTERS_RS points at crates/vexos-ai-viewmodel/src/starters.rs.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const dir = resolve(process.env.EXT ?? 'extension/vexos-assistant@victorytek.github.io');
const rust = readFileSync(resolve(process.env.STARTERS_RS ??
    'crates/vexos-ai-viewmodel/src/starters.rs'), 'utf8');
const {STARTERS, isStarter} = await import(pathToFileURL(join(dir, 'starters.js')));

test('the extension lists the same ids and titles as the app, in the same order', () => {
    const app = [...rust.matchAll(/Starter::new\("(\w+)", "([^"]+)"/g)].map(m => [m[1], m[2]]);
    assert.ok(app.length >= 3);
    assert.deepEqual(STARTERS.map(s => [s.id, s.title]), app);
});

test('isStarter accepts the known ids and nothing else', () => {
    for (const s of STARTERS)
        assert.equal(isStarter(s.id), true);
    for (const bad of ['', 'Diagnose', 'diagnose ', 'x', undefined, null, 0, {}, ['diagnose'], 'diagnose\n'])
        assert.equal(isStarter(bad), false, String(bad));
});

test('every tile has a symbolic icon name', () => {
    for (const s of STARTERS)
        assert.match(s.icon, /^[a-z-]+-symbolic$/);
});
