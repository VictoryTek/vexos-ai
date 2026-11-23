// Run with: node --test tests/extension/   (EXT=<dir> to test a built copy)
import test from 'node:test';
import assert from 'node:assert/strict';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const dir = resolve(process.env.EXT ?? 'extension/vexos-assistant@victorytek.github.io');
const view = await import(pathToFileURL(join(dir, 'usage_view.js')));
const {parseRecord, currentWindows, formatCountdown, formatAge, percentText, isUrgent,
    displayWindows, statusLine, refreshDelaySeconds, shouldRefreshOnOpen, STATUSES, MESSAGES,
    CODEX_MESSAGES, sources, shownSources, MAX_FILE_BYTES, URGENT_PERCENT} = view;

const NOW = 1_800_000_000_000;
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

const record = (over = {}, windows = []) => JSON.stringify({
    schema: 1, updatedAt: NOW, fetchedAt: NOW, probedAt: NOW, nextAt: 0, failures: 0,
    enabled: true, status: 'ok', plan: 'Pro', stale: false, windows, ...over,
});
const win = (id, percent, resetsAt = 0, label = id) => ({id, label, percent, resetsAt});

test('parseRecord: a good record is read field by field', () => {
    const r = parseRecord(record({}, [win('session', 12.5, NOW + HOUR, 'Session'), win('weekly', 40, 0, 'Weekly')]));
    assert.equal(r.enabled, true);
    assert.equal(r.status, 'ok');
    assert.equal(r.plan, 'Pro');
    assert.equal(r.windows.length, 2);
    assert.deepEqual(r.windows[0], {id: 'session', label: 'Session', percent: 12.5, resetsAt: NOW + HOUR});
});

test('parseRecord: anything that is not a record of this version is null', () => {
    const bad = ['', 'null', '[]', '"x"', '{}', 'not json', record({schema: 2}), record({schema: '1'}),
        record({status: 'bogus'}), record({status: 7}), JSON.stringify({schema: 1, status: 'ok'}),
        JSON.stringify({schema: 1, status: 'ok', windows: 'x'}), undefined, null, 5, {}];
    for (const text of bad)
        assert.equal(parseRecord(text), null, String(text).slice(0, 40));
    assert.equal(parseRecord(record() + ' '.repeat(MAX_FILE_BYTES)), null, 'oversize');
});

test('parseRecord: every status the service writes is known', () => {
    for (const status of STATUSES)
        assert.equal(parseRecord(record({status}))?.status, status);
});

test('parseRecord: bad windows are dropped, good ones kept, numbers bounded', () => {
    const r = parseRecord(record({}, [
        win('a', -1), win('b', NaN), {id: 'c', label: 'c', percent: 'high'}, null, 5,
        {id: '', label: 'x', percent: 1}, {id: 'd', label: '', percent: 1},
        win('e', 250, -5), win('f', 10, 1.9),
    ].map(w => (w && w.percent !== undefined && Number.isNaN(w.percent) ? {...w, percent: null} : w))));
    assert.deepEqual(r.windows.map(w => w.id), ['e', 'f']);
    assert.equal(r.windows[0].percent, 100);
    assert.equal(r.windows[0].resetsAt, 0);
    assert.equal(r.windows[1].resetsAt, 1);
    // Never more than 16.
    const many = Array.from({length: 40}, (_, i) => win(`w${i}`, 1));
    assert.equal(parseRecord(record({}, many)).windows.length, 16);
});

test('parseRecord: text from the file is cut to printable ASCII', () => {
    const r = parseRecord(record({plan: 'Max‮ 5x\u0000\n<b>'}, [
        win('scoped:x', 5, 0, 'Fa‮ble\u001b[31m Weekly'.padEnd(200, 'z'))]));
    assert.equal(r.plan, 'Max 5x<b>');
    assert.equal(r.windows[0].label, 'Fable[31m Weekly'.padEnd(60, 'z'));
    for (const text of [r.plan, r.windows[0].label, r.windows[0].id])
        assert.ok(/^[\x20-\x7e]*$/.test(text));
});

test('currentWindows: an elapsed window reads zero, the rest are untouched', () => {
    const r = parseRecord(record({}, [win('a', 80, NOW - 1), win('b', 80, NOW), win('c', 80, NOW + 1), win('d', 80, 0)]));
    const w = currentWindows(r, NOW);
    assert.deepEqual(w.map(x => [x.percent, x.resetsAt]), [[0, 0], [0, 0], [80, NOW + 1], [80, 0]]);
    assert.deepEqual(currentWindows(null, NOW), []);
});

test('formatCountdown', () => {
    const cases = [[NOW + 30_000, '<1m'], [NOW + MIN, '1m'], [NOW + 59 * MIN + 59_000, '59m'],
        [NOW + HOUR, '1h 0m'], [NOW + 3 * HOUR + 11 * MIN, '3h 11m'], [NOW + DAY - 1, '23h 59m'],
        [NOW + DAY, '1d 0h'], [NOW + 4 * DAY + 6 * HOUR, '4d 6h'], [NOW, ''], [NOW - 5, ''],
        [0, ''], [NaN, ''], [Infinity, '']];
    for (const [at, text] of cases)
        assert.equal(formatCountdown(at, NOW), text, String(at - NOW));
});

test('formatAge and percentText', () => {
    assert.equal(formatAge(NOW - 5_000, NOW), 'just now');
    assert.equal(formatAge(NOW - 12 * MIN, NOW), '12 min ago');
    assert.equal(formatAge(NOW - 3 * HOUR, NOW), '3 h ago');
    assert.equal(formatAge(NOW - 2 * DAY, NOW), '2 d ago');
    assert.equal(formatAge(0, NOW), '');
    assert.equal(percentText(12.4), '12%');
    assert.equal(percentText(12.5), '13%');
    assert.equal(percentText(250), '100%');
    assert.equal(percentText(-3), '0%');
});

test('isUrgent: 90% or more in any window, only when plan limits are on', () => {
    assert.equal(URGENT_PERCENT, 90);
    const at = p => parseRecord(record({}, [win('session', 5), win('weekly', p)]));
    assert.equal(isUrgent(at(89.9), NOW), false);
    assert.equal(isUrgent(at(90), NOW), true);
    assert.equal(isUrgent(at(100), NOW), true);
    // A window that has reset is not urgent.
    assert.equal(isUrgent(parseRecord(record({}, [win('weekly', 99, NOW - 1)])), NOW), false);
    assert.equal(isUrgent(parseRecord(record({enabled: false}, [win('weekly', 99)])), NOW), false);
    assert.equal(isUrgent(null, NOW), false);
});

test('displayWindows: Session, Weekly, then scoped, each group in the file order', () => {
    const r = parseRecord(record({}, [win('scoped:b', 1), win('weekly', 2), win('scoped:a', 3), win('session', 4)]));
    assert.deepEqual(displayWindows(r, NOW).map(w => w.id), ['session', 'weekly', 'scoped:b', 'scoped:a']);
});

test('statusLine: one fixed sentence per situation, with the age of kept numbers', () => {
    for (const status of STATUSES) {
        const line = statusLine(parseRecord(record({status}, [win('session', 5)])), NOW);
        // Claude's sign-in is never "no plan": that status is Codex's.
        if (status === 'ok' || status === 'disabled' || status === 'no-plan')
            assert.equal(line, null, status);
        else
            assert.equal(line, MESSAGES[status], status);
    }
    const stale = statusLine(parseRecord(record({status: 'unavailable', stale: true, fetchedAt: NOW - 12 * MIN})), NOW);
    assert.equal(stale, `${MESSAGES.unavailable} Last updated 12 min ago.`);
    assert.equal(statusLine(parseRecord(record({enabled: false, status: 'paused'})), NOW), null);
    assert.equal(statusLine(null, NOW), null);
    for (const text of Object.values(MESSAGES))
        assert.ok(/^[\x20-\x7e]+$/.test(text) && text.length < 100, text);
});

test('refreshDelaySeconds: 15 min, 3 near a limit, 30 s when unreachable, never inside a backoff', () => {
    assert.equal(refreshDelaySeconds(null, NOW), 900);
    assert.equal(refreshDelaySeconds(parseRecord(record({}, [win('session', 5)])), NOW), 900);
    assert.equal(refreshDelaySeconds(parseRecord(record({}, [win('session', 95)])), NOW), 180);
    assert.equal(refreshDelaySeconds(parseRecord(record({status: 'unreachable'})), NOW), 30);
    // The service's own backoff wins.
    assert.equal(refreshDelaySeconds(parseRecord(record({status: 'rate-limited', nextAt: NOW + 20 * MIN})), NOW), 1200);
    assert.equal(refreshDelaySeconds(parseRecord(record({status: 'unreachable', nextAt: NOW + 5_000})), NOW), 30);
    // Plan limits off: the slow default, never faster than 15 s.
    assert.equal(refreshDelaySeconds(parseRecord(record({enabled: false})), NOW), 900);
    for (const r of [null, parseRecord(record({status: 'unreachable'}))])
        assert.ok(refreshDelaySeconds(r, NOW) >= 15);
});

test('shouldRefreshOnOpen: when on and last asked over 2 min ago and not in a backoff, or the record has not caught up', () => {
    const at = over => parseRecord(record(over));
    assert.equal(shouldRefreshOnOpen(at({probedAt: NOW - 3 * MIN}), NOW, true), true);
    assert.equal(shouldRefreshOnOpen(at({probedAt: NOW - MIN}), NOW, true), false);
    assert.equal(shouldRefreshOnOpen(at({probedAt: NOW - 3 * MIN, nextAt: NOW + 1}), NOW, true), false);
    // The setting is on but the record says off, or there is none yet: ask.
    assert.equal(shouldRefreshOnOpen(at({enabled: false, probedAt: 1}), NOW, true), true);
    assert.equal(shouldRefreshOnOpen(null, NOW, true), true);
    // The setting is off: never.
    assert.equal(shouldRefreshOnOpen(at({probedAt: 1}), NOW, false), false);
    assert.equal(shouldRefreshOnOpen(null, NOW, false), false);
    assert.equal(shouldRefreshOnOpen(null, NOW), true, 'on by default');
});

const codex = (over = {}, windows = []) => ({
    status: 'ok', plan: 'Plus', stale: false, fetchedAt: NOW, nextAt: 0, windows, ...over,
});

test('parseRecord: Codex is an optional section read the same careful way', () => {
    const plain = parseRecord(record());
    assert.equal(plain.codex, null);
    const r = parseRecord(record({codex: codex({}, [win('session', 25, NOW + HOUR, 'Session')])}));
    assert.equal(r.codex.status, 'ok');
    assert.equal(r.codex.plan, 'Plus');
    assert.equal(r.codex.enabled, true);
    assert.deepEqual(r.codex.windows, [{id: 'session', label: 'Session', percent: 25, resetsAt: NOW + HOUR}]);
    // A damaged section is no section; Claude's numbers still read.
    for (const bad of [null, 'x', [], {status: 'bogus', windows: []}, {status: 'ok'}, {status: 'ok', windows: 5}]) {
        const damaged = parseRecord(record({codex: bad}, [win('session', 5)]));
        assert.equal(damaged.codex, null, JSON.stringify(bad));
        assert.equal(damaged.windows.length, 1);
    }
});

test('sources: Claude always, Codex when the service found it', () => {
    assert.deepEqual(sources(null), []);
    assert.deepEqual(sources(parseRecord(record())).map(x => x.agent), ['claude']);
    const both = sources(parseRecord(record({codex: codex()})));
    assert.deepEqual(both.map(x => x.agent), ['claude', 'codex']);
    assert.deepEqual(both.map(x => x.name), ['Claude Code', 'Codex']);
});

test('shownSources: an agent that is not signed in is not drawn', () => {
    const out = parseRecord(record({codex: codex({status: 'waiting-for-auth'}, [])}));
    assert.deepEqual(shownSources(out).map(x => x.agent), ['claude']);
    assert.deepEqual(sources(out).map(x => x.agent), ['claude', 'codex']);
    assert.deepEqual(shownSources(parseRecord(record({codex: codex()}))).map(x => x.agent), ['claude', 'codex']);
    const claudeOut = parseRecord(record({status: 'waiting-for-auth'}));
    assert.deepEqual(shownSources(claudeOut).map(x => x.agent), ['claude']);
});

test('isUrgent and the refresh pace follow every agent, not only Claude', () => {
    const calm = parseRecord(record({codex: codex({}, [win('weekly', 95, NOW + HOUR)])}, [win('session', 5)]));
    assert.equal(isUrgent(calm, NOW), true, 'Codex alone is enough');
    assert.equal(refreshDelaySeconds(calm, NOW), 3 * 60);
    const elapsed = parseRecord(record({codex: codex({}, [win('weekly', 95, NOW - MIN)])}, [win('session', 5)]));
    assert.equal(isUrgent(elapsed, NOW), false, 'a reset window is untouched');
    const off = parseRecord(record({enabled: false, codex: codex({}, [win('weekly', 95)])}));
    assert.equal(isUrgent(off, NOW), false);
});

test('statusLine: Codex has its own fixed words and the same age rule', () => {
    for (const status of STATUSES) {
        const r = parseRecord(record({codex: codex({status}, [win('session', 5)])}));
        const line = statusLine(r.codex, NOW, 'codex');
        if (CODEX_MESSAGES[status])
            assert.equal(line, CODEX_MESSAGES[status], status);
        else
            assert.equal(line, null, status);
    }
    const stale = parseRecord(record({codex: codex({status: 'unavailable', stale: true, fetchedAt: NOW - 12 * MIN})}));
    assert.equal(statusLine(stale.codex, NOW, 'codex'), `${CODEX_MESSAGES.unavailable} Last updated 12 min ago.`);
    for (const text of Object.values(CODEX_MESSAGES))
        assert.ok(/^[ -~]+$/.test(text) && text.length < 100, text);
});

test("Claude's backoff does not hold back a healthy Codex", () => {
    const stuck = parseRecord(record({status: 'rate-limited', nextAt: NOW + HOUR, probedAt: NOW - 10 * MIN,
        codex: codex({fetchedAt: NOW - 10 * MIN}, [win('weekly', 95, NOW + DAY)])}, [win('session', 5)]));
    assert.equal(refreshDelaySeconds(stuck, NOW), 3 * 60, 'the near-limit pace, not the hour');
    assert.equal(shouldRefreshOnOpen(stuck, NOW), true, 'Codex is due on open');
    // Both waiting: nothing to ask before the earlier wait ends.
    const both = parseRecord(record({status: 'rate-limited', nextAt: NOW + HOUR, probedAt: NOW - 10 * MIN,
        codex: codex({status: 'unavailable', nextAt: NOW + 20 * MIN, fetchedAt: NOW - 10 * MIN}, [win('weekly', 95, NOW + DAY)])},
    [win('session', 5)]));
    assert.equal(refreshDelaySeconds(both, NOW), 20 * 60);
    assert.equal(shouldRefreshOnOpen(both, NOW), false);
    // Claude's wait alone still holds when there is no Codex.
    const alone = parseRecord(record({status: 'rate-limited', nextAt: NOW + HOUR, probedAt: NOW - 10 * MIN}, [win('session', 5)]));
    assert.equal(refreshDelaySeconds(alone, NOW), 3600);
    assert.equal(shouldRefreshOnOpen(alone, NOW), false);
});

test('pendingLine: checking at first, then the no-answer sentence, never an endless spinner', () => {
    const {pendingLine, PENDING_MS, CHECKING_TEXT, NO_SERVICE_TEXT} = view;
    assert.equal(pendingLine(NOW, 0), CHECKING_TEXT);
    assert.equal(pendingLine(NOW, NOW), CHECKING_TEXT);
    assert.equal(pendingLine(NOW + PENDING_MS - 1, NOW), CHECKING_TEXT);
    assert.equal(pendingLine(NOW + PENDING_MS, NOW), NO_SERVICE_TEXT);
    assert.equal(pendingLine(NOW + 10 * PENDING_MS, NOW), NO_SERVICE_TEXT);
    // Bad input reads as checking, never as a crash.
    assert.equal(pendingLine(NaN, NOW), CHECKING_TEXT);
    assert.ok(NO_SERVICE_TEXT.length < 200 && /^[\x20-\x7e]+$/.test(NO_SERVICE_TEXT.replace(/…/g, '')));
});

test('pendingDelayMs: one redraw is scheduled for the deadline, and none after it (no timer loop)', () => {
    const {pendingDelayMs, PENDING_MS} = view;
    assert.equal(pendingDelayMs(NOW, 0), 0);
    assert.equal(pendingDelayMs(NaN, NOW), 0);
    // Just started: wait for the deadline plus a little.
    assert.equal(pendingDelayMs(NOW, NOW), PENDING_MS + 200);
    // The timer fires after the deadline, so the redraw it causes schedules nothing.
    const fired = NOW + pendingDelayMs(NOW, NOW);
    assert.equal(pendingDelayMs(fired, NOW), 0);
    assert.equal(pendingDelayMs(NOW + PENDING_MS, NOW), 0);
    assert.equal(pendingDelayMs(NOW + 5 * PENDING_MS, NOW), 0);
});

test('needsSignIn: only when no agent is signed in; the choices name all three', () => {
    const {needsSignIn, SIGN_IN_CHOICES} = view;
    assert.equal(needsSignIn(parseRecord(record({status: 'waiting-for-auth'}))), true);
    assert.equal(needsSignIn(parseRecord(record({status: 'waiting-for-auth', codex: codex({status: 'waiting-for-auth'}, [])}))), true);
    assert.equal(needsSignIn(parseRecord(record({status: 'waiting-for-auth', codex: codex()}))), false, 'Codex is signed in');
    assert.equal(needsSignIn(parseRecord(record())), false);
    assert.equal(needsSignIn(null), false);
    assert.deepEqual(SIGN_IN_CHOICES.map(c => c.agent), ['claude', 'codex', 'opencode']);
    for (const c of SIGN_IN_CHOICES) {
        assert.equal(view.AGENT_NAMES[c.agent], c.name);
        assert.ok(view.AGENT_ICONS[c.agent], c.agent);
    }
});
