// Pure logic for the Phase 0 spike: caps, text sanitising and geometry. No
// imports, so node can test it. Nothing here touches a widget.

export const SLIDE_MS = 250;
// Selected text in a reply: St.Label does not read the CSS selection colours, and
// Clutter's default is a white block with light text, so these are set on the
// label's text (the GNOME accent blue with white text).
export const SELECTION_BG = '#3584e4';
export const SELECTION_FG = '#ffffff';
// One message as displayed. The note proposed 64 KiB, but a St.Label paints only
// part of a long text (see the Phase 0 results), and Clutter's text buffer holds
// at most 65535 bytes, which would also cut off the "shortened" mark.
export const MAX_BYTES = 16 * 1024;
export const MAX_LINES = 2000;
export const MAX_RAW = 256 * 1024;    // code units looked at before escaping
export const MAX_ROWS = 200;
// A send is refused above this, never cut. It is below what the entry can hold
// (INPUT_LIMIT_BYTES), so a paste that the entry cut is always too long to send.
export const MAX_SEND_BYTES = 60000;
// Measured in the nested shell (GNOME Shell 50.5): the entry's text buffer keeps
// at most this many bytes, whole characters only, and drops the rest of a paste
// without a word.
export const INPUT_LIMIT_BYTES = 65534;
// The longest whole character is 4 bytes, so a text this close to the limit may
// have lost its tail.
const INPUT_FULL_BYTES = INPUT_LIMIT_BYTES - 3;
export const MAX_INPUT_CHARS = 72 * 1024;
export const INPUT_MAX_LINES = 8;

export const WIDTH_FRACTION = 0.5;
export const MIN_WIDTH = 720;
export const MAX_WIDTH = 960;
export const HEIGHT_FRACTION = 0.6;
export const MIN_HEIGHT = 520;
export const MAX_HEIGHT = 1000;

// Controls, bidirectional controls, line/paragraph separators and lone
// surrogates are shown as text, never interpreted. Newline and tab stay.
function mustEscape(cp) {
    return (cp < 0x20 && cp !== 0x0a && cp !== 0x09) ||
        cp === 0x7f || (cp >= 0x80 && cp <= 0x9f) ||
        cp === 0x061c || cp === 0x200e || cp === 0x200f ||
        cp === 0x2028 || cp === 0x2029 ||
        (cp >= 0x202a && cp <= 0x202e) ||
        (cp >= 0x2066 && cp <= 0x2069) ||
        (cp >= 0xd800 && cp <= 0xdfff);
}

// Text made safe to show: CRLF becomes LF, every character in mustEscape
// becomes \u{hex}. With `backslash`, a literal backslash is doubled too, so an
// escape in the output cannot be mistaken for one we wrote (card style).
export function visible(text, {backslash = false} = {}) {
    if (typeof text !== 'string')
        return '';
    let out = '';
    const s = text.replace(/\r\n/g, '\n');
    for (let i = 0; i < s.length;) {
        const cp = s.codePointAt(i);
        i += cp > 0xffff ? 2 : 1;
        if (mustEscape(cp))
            out += `\\u{${cp.toString(16)}}`;
        else if (backslash && cp === 0x5c)
            out += '\\\\';
        else
            out += String.fromCodePoint(cp);
    }
    return out;
}

function utf8Len(cp) {
    return cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
}

// UTF-8 size of a string, counting a lone surrogate as 3 (as an encoder
// would for its replacement character).
export function utf8Length(text) {
    let n = 0;
    for (let i = 0; i < text.length;) {
        const cp = text.codePointAt(i);
        i += cp > 0xffff ? 2 : 1;
        n += utf8Len(cp);
    }
    return n;
}

// What is drawn for one message: only the first MAX_RAW units are read, then
// escaped, then cut to MAX_BYTES (never inside a character) and MAX_LINES, and
// a visible mark says how much was left out, in UTF-16 units of the escaped text
// (not original characters: one escaped control character counts as several).
export function capText(text) {
    const raw = typeof text === 'string' ? text : '';
    const kept = raw.length > MAX_RAW ? raw.slice(0, MAX_RAW) : raw;
    const esc = visible(kept);
    let end = 0;
    let bytes = 0;
    while (end < esc.length) {
        const cp = esc.codePointAt(end);
        const n = utf8Len(cp);
        if (bytes + n > MAX_BYTES)
            break;
        bytes += n;
        end += cp > 0xffff ? 2 : 1;
    }
    let lines = 1;
    for (let i = 0; i < end; i++) {
        if (esc.charCodeAt(i) === 0x0a && ++lines > MAX_LINES) {
            end = i;
            break;
        }
    }
    const hidden = (raw.length - kept.length) + (esc.length - end);
    const shown = esc.slice(0, end);
    return hidden > 0 ? `${shown}\n… shortened: ${hidden} units not shown` : shown;
}

// The newest MAX_ROWS rows.
export function capRows(rows) {
    return rows.length > MAX_ROWS ? rows.slice(rows.length - MAX_ROWS) : rows.slice();
}

// 'send', 'empty' (nothing but whitespace) or 'too-long' (kept in the box).
export function inputVerdict(text) {
    if (typeof text !== 'string' || text.trim() === '')
        return 'empty';
    // A character is at least one byte per code unit, so this is a cheap bound.
    if (text.length > MAX_SEND_BYTES || utf8Length(text) > MAX_SEND_BYTES)
        return 'too-long';
    return 'send';
}

// What to tell the user about the text in the box: 'none', 'long' (over the
// send limit) or 'full' (so near the entry's limit that a paste may have been cut).
export function inputNotice(text) {
    if (typeof text !== 'string')
        return 'none';
    const n = utf8Length(text);
    if (n >= INPUT_FULL_BYTES)
        return 'full';
    return n > MAX_SEND_BYTES ? 'long' : 'none';
}

// The notice with memory. `cut` says an earlier text in this box was cut by the
// entry; it stays set, whatever the text shrinks to, until the box is empty
// (emptied by the user or by a send). Sending is not affected: this only says
// what to show. Kinds: 'none', 'long', 'full', 'cut' (shorter now, can be sent)
// and 'cut-long' (shorter now, still over the send limit).
export function inputNoticeState(cut, text) {
    if (typeof text !== 'string' || text === '')
        return {cut: false, kind: 'none'};
    const kind = inputNotice(text);
    if (kind === 'full')
        return {cut: true, kind};
    if (cut)
        return {cut: true, kind: kind === 'long' ? 'cut-long' : 'cut'};
    return {cut: false, kind};
}

const num = (v, d = 0) => Number.isFinite(v) ? v : d;

function fit(fraction, min, max, available) {
    const wanted = Math.min(max, Math.max(min, Math.round(available * fraction)));
    return Math.min(wanted, Math.max(0, available));
}

// The drop-down: centred under the top bar, flush with its bottom edge.
// hiddenY puts the whole panel above the bar, as Looking Glass does.
export function panelRect(monitor, barHeight) {
    const bar = Math.max(0, num(barHeight));
    const mx = num(monitor.x), my = num(monitor.y);
    const mw = Math.max(0, num(monitor.width)), mh = Math.max(0, num(monitor.height));
    const width = fit(WIDTH_FRACTION, MIN_WIDTH, MAX_WIDTH, mw);
    const height = fit(HEIGHT_FRACTION, MIN_HEIGHT, MAX_HEIGHT, Math.max(0, mh - bar));
    return {
        x: mx + Math.round((mw - width) / 2),
        width,
        height,
        hiddenY: my + bar - height,
        targetY: my + bar,
    };
}

// The new scroll value that shows [top, bottom] in a page, moving as little
// as possible. A bad number leaves the value alone.
export function scrollToShow(value, pageSize, top, bottom) {
    if (![value, pageSize, top, bottom].every(Number.isFinite))
        return num(value);
    if (top < value)
        return top;
    if (bottom > value + pageSize)
        return bottom - pageSize;
    return value;
}

// The input box's height: its text's natural height, between one line and the
// cap.
export function inputHeight(natural, minHeight, maxHeight) {
    const lo = Math.max(0, num(minHeight));
    const hi = Math.max(lo, num(maxHeight, lo));
    return Math.min(hi, Math.max(lo, num(natural, lo)));
}
