// The docked window's size and place, and the shared "window-mode" value, as
// pure functions with no Shell imports, so node can test them. Sizes are in
// logical pixels. Nothing here touches a window.

export const DOCKED = 'docked';
export const FLOATING = 'floating';

export const WIDTH_FRACTION = 0.5;
export const MIN_WIDTH = 720;
export const MAX_WIDTH = 960;
export const HEIGHT_FRACTION = 0.6;
export const MIN_HEIGHT = 520;
export const MAX_HEIGHT = 1000;

// Exactly "floating" is floating; anything else (unknown, empty, huge, with
// control characters, not a string) is the default, docked.
export function parseMode(value) {
    return value === FLOATING ? FLOATING : DOCKED;
}

function fit(fraction, min, max, available) {
    const wanted = Math.min(max, Math.max(min, Math.round(available * fraction)));
    return Math.min(wanted, Math.max(0, available));
}

// Centred in the work area, flush with its top edge.
export function dockedRect(area) {
    const width = fit(WIDTH_FRACTION, MIN_WIDTH, MAX_WIDTH, area.width);
    const height = fit(HEIGHT_FRACTION, MIN_HEIGHT, MAX_HEIGHT, area.height);
    return {
        x: area.x + Math.round((area.width - width) / 2),
        y: area.y,
        width,
        height,
    };
}

// The remembered normal size, centred in the work area and capped to it.
export function floatingRect(area, normal) {
    const width = Math.min(Math.max(0, normal.width), Math.max(0, area.width));
    const height = Math.min(Math.max(0, normal.height), Math.max(0, area.height));
    return {
        x: area.x + Math.round((area.width - width) / 2),
        y: area.y + Math.round((area.height - height) / 2),
        width,
        height,
    };
}

// The app's own default size (crates/vexos-ai-panel: default_width/height).
export const DEFAULT_NORMAL = {width: 760, height: 820};

// The size to go back to when floating: the window's size at its first show,
// unless that is already the docked size (the app remembers its last size), in
// which case the app's default.
export function normalSize(frame, area) {
    const docked = dockedRect(area);
    if (frame.width === docked.width && frame.height === docked.height)
        return {...DEFAULT_NORMAL};
    return {width: frame.width, height: frame.height};
}

// The slide down from the top bar.
export const SLIDE_MS = 250;
// Clutter.AnimationMode name, mapped to the enum in extension.js.
export const SLIDE_EASING = 'EASE_OUT_QUAD';
// If the window has not reported its move by then, show it anyway.
export const FALLBACK_MS = 400;

// translation_y that puts the whole window above the screen's top edge when it
// is docked at the work area's top (the top bar sits between the two).
export function slideStart(height, areaY, screenY) {
    const h = Number.isFinite(height) ? Math.max(0, height) : 0;
    const bar = Number.isFinite(areaY - screenY) ? Math.max(0, areaY - screenY) : 0;
    return -(h + bar);
}

// What to do with the hidden window: 'slide' once its move has landed, 'show'
// (no slide, just visible) when it never does, 'abort' if it is gone, else
// 'wait'. Gone wins over everything, a landed move over the timeout.
export function slideDecision({landed, gone, timedOut}) {
    if (gone)
        return 'abort';
    if (landed)
        return 'slide';
    return timedOut ? 'show' : 'wait';
}
