// The spike's whole conversation: hard-coded, no helper, no agent, no files.
// `kind` picks a style only: 'text', 'code' (monospace, wraps by character).

const para = 'The quick brown fox jumps over the lazy dog. ';

export const CONVERSATION = [
    {role: 'you', kind: 'text', text: 'Why did my last rebuild take so long?'},
    {role: 'assistant', kind: 'text', text: 'Fake reply 1. This is plain text from a hard-coded list; nothing was asked of an agent.'},
    {role: 'assistant', kind: 'text',
        text: `Very long message (about 200 KiB, capped on display):\n${para.repeat(4600)}`},
    {role: 'assistant', kind: 'code',
        text: Array.from({length: 400}, (_, i) => `line ${String(i + 1).padStart(3, '0')}: const value${i} = compute(${i}, 'argument');`).join('\n')},
    {role: 'assistant', kind: 'code',
        text: `${'| column '.repeat(300)}|`},
    {role: 'assistant', kind: 'text',
        text: 'Markup must stay text: <b>bold</b> <i>it</i> &amp; &lt;tag&gt; <span foreground="red">red</span> <a href="https://example.invalid">link</a> %s %d {0} ${x}'},
    {role: 'assistant', kind: 'text',
        text: 'Controls: RLO[\u202e]abc LRI[\u2066] RLM[\u200f] ESC[\u001b[31m] BEL[\u0007] NUL[\u0000] CR[\r] CRLF[\r\n] LS[\u2028] DEL[\u007f] C1[\u0085] lone surrogate[\ud800] backslash[\\u{41}]'},
    {role: 'you', kind: 'text', text: 'Short question.\nWith a second line.'},
];

// One fake approval card. The buttons only change the status line.
export const CARD = {
    title: 'Fake approval: run a command',
    command: 'echo "not run"; # \u202e hidden-direction \u2069 \\u{41}',
};
