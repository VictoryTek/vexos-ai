#!/usr/bin/env bash
# keys.sh COMBO...: key presses into the nested shell through a virtual
# keyboard. COMBO is like super+F10, shift+Return, ctrl+a, Escape, or
# text:some lowercase words (letters, digits, space). Prints the monotonic
# time (microseconds) taken just before the last key.
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
args=$(printf '%s\n' "$@" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().split("\n")[:-1]))')
exec "$(dirname "${BASH_SOURCE[0]}")/ev.sh" "
const {Clutter, GLib} = imports.gi;
globalThis.__kbd ??= Clutter.get_default_backend().get_default_seat()
    .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
const kbd = globalThis.__kbd;
const codes = {Escape: 1, BackSpace: 14, Tab: 15, Return: 28, ctrl: 29, shift: 42, space: 57,
  F10: 68, Home: 102, Up: 103, Left: 105, Right: 106, End: 107, Down: 108, Delete: 111, super: 125,
  ' ': 57};
'qwertyuiop'.split('').forEach((c, i) => codes[c] = 16 + i);
'asdfghjkl'.split('').forEach((c, i) => codes[c] = 30 + i);
'zxcvbnm'.split('').forEach((c, i) => codes[c] = 44 + i);
'1234567890'.split('').forEach((c, i) => codes[c] = 2 + i);
let t0 = 0;
const tap = (mods, key) => {
  for (const m of mods) kbd.notify_key(GLib.get_monotonic_time(), codes[m], Clutter.KeyState.PRESSED);
  t0 = GLib.get_monotonic_time();
  kbd.notify_key(t0, codes[key], Clutter.KeyState.PRESSED);
  kbd.notify_key(GLib.get_monotonic_time(), codes[key], Clutter.KeyState.RELEASED);
  for (const m of mods.slice().reverse()) kbd.notify_key(GLib.get_monotonic_time(), codes[m], Clutter.KeyState.RELEASED);
};
for (const a of $args) {
  if (a.startsWith('text:')) { for (const ch of a.slice(5)) tap([], ch); }
  else { const parts = a.split('+'); tap(parts.slice(0, -1), parts.at(-1)); }
}
t0;"
