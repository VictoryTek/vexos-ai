#!/usr/bin/env python3
"""Mutation check for the spike's logic and source scans.

  mutate.py make DIR   copy the extension once per mutation into DIR/mNN (DIR must
                       not exist; nothing is deleted) and apply one mutation each
  mutate.py run DIR    run the node tests against every copy; every one must fail

Run `make` and `run` as separate steps. DIR is best under target/.
"""
import os, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, '..', 'vexos-shell-spike@victorytek.github.io')
TEST = os.path.join(HERE, 'logic.test.mjs')

M=[
 ('logic.js','bytes + n > MAX_BYTES','bytes + n >= MAX_BYTES'),
 ('logic.js','++lines > MAX_LINES','++lines >= MAX_LINES'),
 ('logic.js','cp >= 0x202a && cp <= 0x202e','cp >= 0x202b && cp <= 0x202e'),
 ('logic.js','(cp >= 0xd800 && cp <= 0xdfff)','(cp >= 0xd800 && cp <= 0xdbff)'),
 ('logic.js','else if (backslash && cp === 0x5c)','else if (false && cp === 0x5c)'),
 ("logic.js","replace(/\\r\\n/g, '\\n')","replace(/\\r\\n/g, '\\r\\n')"),
 ('logic.js','rows.slice(rows.length - MAX_ROWS)','rows.slice(0, MAX_ROWS)'),
 ('logic.js','if (top < value)','if (top > value)'),
 ('logic.js','utf8Length(text) > MAX_SEND_BYTES','utf8Length(text) >= MAX_SEND_BYTES'),
 ('logic.js','cp === 0x061c ||','false ||'),
 ('logic.js','cp >= 0x80 && cp <= 0x9f','cp >= 0x80 && cp <= 0x9e'),
 ('logic.js','raw.slice(0, MAX_RAW) : raw','raw.slice(0, MAX_RAW + 1000) : raw'),
 ('logic.js','(raw.length - kept.length) + ',''),
 ('logic.js','Math.min(hi, Math.max(lo, num(natural, lo)))','Math.max(lo, num(natural, lo))'),
 ('logic.js','hiddenY: my + bar - height','hiddenY: my - height'),
 ('logic.js','cp < 0x800 ? 2','cp < 0x800 ? 3'),
 ('logic.js','return bottom - pageSize;','return bottom;'),
 ('logic.js',"cp === 0x2028 || cp === 0x2029 ||","cp === 0x2028 ||"),
 ('logic.js','(cp >= 0x2066 && cp <= 0x2069)','(cp >= 0x2066 && cp <= 0x2068)'),
 ('logic.js','cp < 0x20 && cp !== 0x0a','cp < 0x20 && cp !== 0x0d && cp !== 0x0a'),
 ('extension.js',"ellipsize: Pango.EllipsizeMode.NONE});\n        row.add_child(label);","ellipsize: Pango.EllipsizeMode.NONE});\n        label.clutter_text.use_markup = true;\n        row.add_child(label);"),
 ('extension.js','Main.popModal(grab);','void grab;'),
 ('extension.js','close.connect(\'clicked\', this._guard(() => this._close()));','close.connect(\'clicked\', () => this._close());'),
 ('extension.js','handler error: ${e?.name ?? \'Error\'}','handler error: ${e.message}'),
 ('extension.js',"global.stage.connect('after-paint'","global.stage.connect('captured-event'"),
 ('extension.js','St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, shown)','St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, shown)'),
 ('logic.js','return hidden > 0 ?','return hidden > 1000000 ?'),
 ('logic.js','export const MAX_BYTES = 16 * 1024;','export const MAX_BYTES = 64 * 1024;'),
 ('extension.js','global.stage.disconnect(id);','void id;'),
 ('extension.js','Main.wm.removeKeybinding(SHORTCUT);','void SHORTCUT;'),
]


def make(out):
    os.makedirs(out)
    for i, (f, a, b) in enumerate(M, 1):
        d = os.path.join(out, f'm{i:02d}')
        shutil.copytree(SRC, d)
        p = os.path.join(d, f)
        s = open(p, encoding='utf-8').read()
        assert s.count(a) == 1, (i, a, s.count(a))
        open(p, 'w', encoding='utf-8').write(s.replace(a, b))
    print(len(M), 'mutants in', out)


def run(out):
    survivors = []
    dirs = sorted(os.listdir(out))
    for name in dirs:
        env = dict(os.environ, SPIKE=os.path.abspath(os.path.join(out, name)))
        r = subprocess.run(['node', '--test', TEST], env=env, capture_output=True)
        if r.returncode == 0:
            survivors.append(name)
    print(f'killed {len(dirs) - len(survivors)} of {len(dirs)}; survivors: {survivors or "none"}')
    sys.exit(1 if survivors else 0)


if __name__ == '__main__' and len(sys.argv) == 3 and sys.argv[1] in ('make', 'run'):
    {'make': make, 'run': run}[sys.argv[1]](sys.argv[2])
else:
    sys.exit(__doc__)
