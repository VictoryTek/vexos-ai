// What the top-bar panel shows about plan limits, as pure functions. No
// imports, so the tests run them under node.
//
// The input is the usage service's record (crates/vexos-ai-usage,
// $XDG_STATE_HOME/vexos-ai/usage.json): numbers and fixed words. It is read
// from disk, so every field is checked here and nothing in it is trusted.

export const SCHEMA = 1;
export const MAX_FILE_BYTES = 64 * 1024;
export const URGENT_PERCENT = 90;

export const STATUSES = ['ok', 'disabled', 'waiting-for-auth', 'paused',
    'signin-expired', 'unavailable', 'rate-limited', 'unreachable', 'format-changed'];

// The panel's own words, one fixed sentence for each situation.
export const MESSAGES = {
    'waiting-for-auth': 'Waiting for a sign-in. Start Claude Code and sign in.',
    'paused': "Limits paused: Claude Code's access token lapsed. Start Claude Code to renew it.",
    'signin-expired': 'Sign-in expired. Start Claude Code, or run `claude auth login`, to sign in again.',
    'unavailable': 'Limits are unavailable right now.',
    'rate-limited': 'Anthropic is rate limiting checks. Showing the last known limits.',
    'unreachable': "Can't reach Anthropic. Retrying shortly.",
    'format-changed': 'Anthropic changed the usage format, so limits are unavailable.',
};

const MINUTE = 60 * 1000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

// How often the service is asked: every 15 minutes, every 3 near a limit, and
// again in 30 s when nothing could be reached.
export const REFRESH_SECONDS = 15 * 60;
export const NEAR_LIMIT_REFRESH_SECONDS = 3 * 60;
export const RETRY_SECONDS = 30;
export const MIN_REFRESH_SECONDS = 15;
// Opening the panel asks again only when the last request is older than this.
export const OPEN_REFRESH_AFTER_MS = 2 * MINUTE;

const clean = (text, max) => String(text).replace(/[^\x20-\x7e]/g, '').slice(0, max);
const whole = n => (Number.isFinite(n) && n >= 0 ? Math.floor(n) : 0);

// Reads the service's file. Returns null for anything that is not a record
// this version understands, so the panel shows "no data" rather than guessing.
export function parseRecord(text) {
    if (typeof text !== 'string' || text.length === 0 || text.length > MAX_FILE_BYTES)
        return null;
    let v;
    try {
        v = JSON.parse(text);
    } catch {
        return null;
    }
    if (v === null || typeof v !== 'object' || Array.isArray(v) || v.schema !== SCHEMA)
        return null;
    if (!STATUSES.includes(v.status) || !Array.isArray(v.windows))
        return null;
    const windows = [];
    for (const w of v.windows.slice(0, 16)) {
        if (w === null || typeof w !== 'object')
            continue;
        const percent = typeof w.percent === 'number' ? w.percent : NaN;
        if (!Number.isFinite(percent) || percent < 0)
            continue;
        const id = clean(w.id ?? '', 60);
        const label = clean(w.label ?? '', 60).trim();
        if (id === '' || label === '')
            continue;
        windows.push({id, label, percent: Math.min(100, percent), resetsAt: whole(w.resetsAt)});
    }
    return {
        enabled: v.enabled === true,
        status: v.status,
        plan: clean(v.plan ?? '', 24).trim(),
        stale: v.stale === true,
        updatedAt: whole(v.updatedAt),
        fetchedAt: whole(v.fetchedAt),
        probedAt: whole(v.probedAt),
        nextAt: whole(v.nextAt),
        windows,
    };
}

// A window whose reset has passed is untouched, not unknown: it reads 0%.
export function currentWindows(record, nowMs) {
    if (!record)
        return [];
    return record.windows.map(w =>
        w.resetsAt > 0 && w.resetsAt <= nowMs ? {...w, percent: 0, resetsAt: 0} : w);
}

// The time until a reset: "12m", "3h 11m", "4d 6h". Empty when unknown or past.
export function formatCountdown(resetsAt, nowMs) {
    const ms = resetsAt - nowMs;
    if (!Number.isFinite(ms) || resetsAt <= 0 || ms <= 0)
        return '';
    if (ms < MINUTE)
        return '<1m';
    if (ms < HOUR)
        return `${Math.floor(ms / MINUTE)}m`;
    if (ms < DAY)
        return `${Math.floor(ms / HOUR)}h ${Math.floor((ms % HOUR) / MINUTE)}m`;
    return `${Math.floor(ms / DAY)}d ${Math.floor((ms % DAY) / HOUR)}h`;
}

// "just now", "12 min ago", "3 h ago", "2 d ago"; empty when never.
export function formatAge(at, nowMs) {
    const ms = nowMs - at;
    if (!Number.isFinite(ms) || at <= 0)
        return '';
    if (ms < MINUTE)
        return 'just now';
    if (ms < HOUR)
        return `${Math.floor(ms / MINUTE)} min ago`;
    if (ms < DAY)
        return `${Math.floor(ms / HOUR)} h ago`;
    return `${Math.floor(ms / DAY)} d ago`;
}

export function percentText(percent) {
    return `${Math.round(Math.min(100, Math.max(0, percent)))}%`;
}

// The icon turns urgent when any window is at 90% or more.
export function isUrgent(record, nowMs) {
    if (!record || !record.enabled)
        return false;
    return currentWindows(record, nowMs).some(w => w.percent >= URGENT_PERCENT);
}

// Sorted for display: Session, Weekly, then model-scoped windows.
export function displayWindows(record, nowMs) {
    const rank = w => (w.id === 'session' ? 0 : w.id === 'weekly' ? 1 : 2);
    return currentWindows(record, nowMs)
        .map((w, i) => ({w, i}))
        .sort((a, b) => rank(a.w) - rank(b.w) || a.i - b.i)
        .map(({w}) => w);
}

// The sentence under the meters, if the situation needs one.
export function statusLine(record, nowMs) {
    if (!record || !record.enabled)
        return null;
    const text = MESSAGES[record.status];
    if (!text)
        return record.stale ? `Last updated ${formatAge(record.fetchedAt, nowMs)}.` : null;
    const age = record.stale && record.fetchedAt > 0
        ? ` Last updated ${formatAge(record.fetchedAt, nowMs)}.` : '';
    return text + age;
}

// Seconds until the next request. Never under 15, and never before the
// service's own backoff ends.
export function refreshDelaySeconds(record, nowMs) {
    let delay = REFRESH_SECONDS;
    if (record && record.enabled) {
        if (currentWindows(record, nowMs).some(w => w.percent >= URGENT_PERCENT))
            delay = NEAR_LIMIT_REFRESH_SECONDS;
        if (record.status === 'unreachable')
            delay = RETRY_SECONDS;
        if (record.nextAt > nowMs)
            delay = Math.max(delay, Math.ceil((record.nextAt - nowMs) / 1000));
    }
    return Math.max(MIN_REFRESH_SECONDS, delay);
}

// Opening the panel asks for fresh numbers when the last request is old, or
// when plan limits are on but the record does not say so yet (the switch was
// just turned on, or the record was cleared).
export function shouldRefreshOnOpen(record, nowMs, settingOn = true) {
    if (!settingOn)
        return false;
    if (!record || !record.enabled)
        return true;
    return nowMs - record.probedAt > OPEN_REFRESH_AFTER_MS && nowMs >= record.nextAt;
}
