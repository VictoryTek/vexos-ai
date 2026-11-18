// What the top-bar panel shows about plan limits, as pure functions. No
// imports, so the tests run them under node.
//
// The input is the usage service's record (crates/vexos-ai-usage,
// $XDG_STATE_HOME/vexos-ai/usage.json): numbers and fixed words. It is read
// from disk, so every field is checked here and nothing in it is trusted.
// The record's own fields are Claude's; an optional `codex` section holds
// Codex's, in the same shape.

export const SCHEMA = 1;
export const MAX_FILE_BYTES = 64 * 1024;
export const URGENT_PERCENT = 90;

export const STATUSES = ['ok', 'disabled', 'waiting-for-auth', 'paused',
    'signin-expired', 'unavailable', 'rate-limited', 'unreachable', 'format-changed', 'no-plan'];

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

// The same for Codex, whose few situations read differently.
export const CODEX_MESSAGES = {
    'waiting-for-auth': 'Waiting for a sign-in. Run `codex login`.',
    'unavailable': 'Codex limits are unavailable right now.',
    'format-changed': 'Codex changed its usage format, so limits are unavailable.',
    'no-plan': 'This Codex sign-in has no plan limits.',
};

export const AGENT_NAMES = {claude: 'Claude Code', codex: 'Codex'};

// With plan limits on and no record yet, the panel says it is checking. If
// the service has not written anything after this long, it says so instead of
// waiting for ever, because a service that never started looks the same.
export const PENDING_MS = 12 * 1000;
export const CHECKING_TEXT = 'Checking your limits…';
export const NO_SERVICE_TEXT = "The usage service did not answer. Log out and back in; if this stays, check `journalctl --user -g vexos-ai-usage`.";

// The sentence for "on, but no record": checking, then the failure. `since` is
// when the panel began waiting (ms), 0 when it has not.
export function pendingLine(nowMs, since) {
    return since > 0 && nowMs - since >= PENDING_MS ? NO_SERVICE_TEXT : CHECKING_TEXT;
}

// Milliseconds until the panel should redraw to show the no-answer sentence;
// 0 when that time has come (nothing left to wait for, so no timer).
export function pendingDelayMs(nowMs, since) {
    if (!(since > 0) || !Number.isFinite(nowMs))
        return 0;
    const left = PENDING_MS - (nowMs - since);
    return left > 0 ? Math.ceil(left) + 200 : 0;
}

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
    const enabled = v.enabled === true;
    return {
        ...readSource(v, enabled),
        enabled,
        updatedAt: whole(v.updatedAt),
        probedAt: whole(v.probedAt),
        codex: v.codex !== null && typeof v.codex === 'object' && !Array.isArray(v.codex) && STATUSES.includes(v.codex.status) && Array.isArray(v.codex.windows)
            ? {...readSource(v.codex, enabled), enabled} : null,
    };
}

// The fields every source has, whichever agent it is: Claude's are the
// record's own, Codex's are its `codex` section.
function readSource(v, enabled) {
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
        enabled,
        status: v.status,
        plan: clean(v.plan ?? '', 24).trim(),
        stale: v.stale === true,
        fetchedAt: whole(v.fetchedAt),
        nextAt: whole(v.nextAt),
        windows,
    };
}

// The agents with something to show: Claude's always (it is the record), then
// Codex's when the service found Codex.
export function sources(record) {
    if (!record)
        return [];
    const list = [{agent: 'claude', name: AGENT_NAMES.claude, data: record}];
    if (record.codex)
        list.push({agent: 'codex', name: AGENT_NAMES.codex, data: record.codex});
    return list;
}

// What the panel draws: the same, but an agent that is not signed in is left
// out (Claude keeps its sign-in sentence, since it is the record itself).
export function shownSources(record) {
    return sources(record).filter(({agent, data}) =>
        agent === 'claude' || data.status !== 'waiting-for-auth');
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

// The icon turns urgent when any window of any agent is at 90% or more.
export function isUrgent(record, nowMs) {
    if (!record || !record.enabled)
        return false;
    return sources(record).some(({data}) =>
        currentWindows(data, nowMs).some(w => w.percent >= URGENT_PERCENT));
}

// Sorted for display: Session, Weekly, then model-scoped windows.
export function displayWindows(record, nowMs) {
    const rank = w => (w.id === 'session' ? 0 : w.id === 'weekly' ? 1 : 2);
    return currentWindows(record, nowMs)
        .map((w, i) => ({w, i}))
        .sort((a, b) => rank(a.w) - rank(b.w) || a.i - b.i)
        .map(({w}) => w);
}

// The sentence under an agent's meters, if the situation needs one.
export function statusLine(record, nowMs, agent = 'claude') {
    if (!record || !record.enabled)
        return null;
    const text = (agent === 'codex' ? CODEX_MESSAGES : MESSAGES)[record.status];
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
        if (isUrgent(record, nowMs))
            delay = NEAR_LIMIT_REFRESH_SECONDS;
        if (record.status === 'unreachable')
            delay = RETRY_SECONDS;
        // A request asks every agent, and each keeps its own backoff, so only
        // when all of them are waiting is there nothing to ask before the
        // earliest of those waits ends.
        const waits = sources(record).map(({data}) => data.nextAt);
        if (waits.every(at => at > nowMs))
            delay = Math.max(delay, Math.ceil((Math.min(...waits) - nowMs) / 1000));
    }
    return Math.max(MIN_REFRESH_SECONDS, delay);
}

// Opening the panel asks for fresh numbers when an agent's last request is old, or
// when plan limits are on but the record does not say so yet (the switch was
// just turned on, or the record was cleared).
export function shouldRefreshOnOpen(record, nowMs, settingOn = true) {
    if (!settingOn)
        return false;
    if (!record || !record.enabled)
        return true;
    // Due when any agent was last asked long ago and is not in a backoff.
    return sources(record).some(({agent, data}) => {
        const asked = agent === 'codex' ? data.fetchedAt : record.probedAt;
        return nowMs - asked > OPEN_REFRESH_AFTER_MS && nowMs >= data.nextAt;
    });
}
