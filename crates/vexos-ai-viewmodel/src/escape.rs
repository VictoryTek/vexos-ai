//! Text that must be shown as it is. Agent text on an approval card (the
//! command, a path, a diff) is shown to the user so they can decide, so what
//! they read has to be what is there: a control or invisible character is shown
//! as `\u{hex}`, and nothing is stripped, normalised or hidden.
//!
//! A real escape and the same characters typed as text must never look alike.
//! A run of backslashes is written twice over when what follows it could be
//! mistaken for an escape: the text `u{` (the start of one) or a character that
//! is escaped here (whose escape starts with one more backslash). Any other
//! backslash is left alone, so `grep '\.nix'` reads as typed. Every output
//! decodes to exactly one input.

/// Characters that hide, reorder or break text on screen without drawing
/// anything: soft hyphen, combining grapheme joiner, bidi marks, joiners and
/// zero-width spaces, line and paragraph separators, word joiner and the
/// invisible operators, deprecated format controls, variation selectors, the
/// byte order mark, interlinear annotation marks and tag characters.
pub(crate) fn is_hidden(c: char) -> bool {
    matches!(c,
        '\u{AD}' | '\u{34F}' | '\u{61C}' | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}'
        | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
        | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0000}'..='\u{E007F}' | '\u{E0100}'..='\u{E01EF}')
}

/// A character shown as its code: a control other than newline and tab, or a
/// hidden one.
fn needs_escape(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t') || is_hidden(c)
}

/// `text` with every control (except newline and tab) and hidden character
/// shown as `\u{hex}`.
pub fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '\\' {
            let run = rest.bytes().take_while(|&b| b == b'\\').count();
            rest = &rest[run..];
            let mut next = rest.chars();
            let escape_follows = match next.next() {
                Some(n) if needs_escape(n) => true,
                Some('u') => next.next() == Some('{'),
                _ => false,
            };
            out.push_str(&"\\".repeat(if escape_follows { run * 2 } else { run }));
            continue;
        }
        if needs_escape(c) {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(c: char) -> String {
        format!("\\u{{{:x}}}", c as u32)
    }

    /// The inverse, for the tests only: the text `visible` was given.
    fn unescape(shown: &str) -> String {
        let c: Vec<char> = shown.chars().collect();
        let (mut out, mut i) = (String::new(), 0);
        while i < c.len() {
            if c[i] != '\\' {
                out.push(c[i]);
                i += 1;
                continue;
            }
            let mut j = i;
            while j < c.len() && c[j] == '\\' {
                j += 1;
            }
            let run = j - i;
            let brace = c.get(j) == Some(&'u') && c.get(j + 1) == Some(&'{');
            if !brace {
                out.extend(std::iter::repeat_n('\\', run));
                i = j;
            } else if run % 2 == 0 {
                out.extend(std::iter::repeat_n('\\', run / 2));
                i = j;
            } else {
                out.extend(std::iter::repeat_n('\\', (run - 1) / 2));
                let end = c[j..].iter().position(|&x| x == '}').expect("closed") + j;
                let hex: String = c[j + 2..end].iter().collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                i = end + 1;
            }
        }
        out
    }

    /// The first and last character of every escaped range, and one inside.
    const ESCAPED: &[char] = &[
        '\u{0}',
        '\u{1}',
        '\u{8}',
        '\u{b}',
        '\u{c}',
        '\u{d}',
        '\u{1b}',
        '\u{1f}',
        '\u{7f}',
        '\u{80}',
        '\u{85}',
        '\u{9f}',
        '\u{ad}',
        '\u{34f}',
        '\u{61c}',
        '\u{180b}',
        '\u{180e}',
        '\u{180f}',
        '\u{200b}',
        '\u{200d}',
        '\u{200f}',
        '\u{2028}',
        '\u{2029}',
        '\u{202a}',
        '\u{202e}',
        '\u{2060}',
        '\u{2064}',
        '\u{2066}',
        '\u{2069}',
        '\u{206a}',
        '\u{206f}',
        '\u{fe00}',
        '\u{fe0f}',
        '\u{feff}',
        '\u{fff9}',
        '\u{fffb}',
        '\u{e0000}',
        '\u{e0041}',
        '\u{e007f}',
        '\u{e0100}',
        '\u{e01ef}',
    ];

    /// Just outside each range: visible, so left alone.
    const KEPT: &[char] = &[
        '\u{20}',
        '\u{7e}',
        '\u{a0}',
        '\u{ac}',
        '\u{ae}',
        '\u{34e}',
        '\u{350}',
        '\u{61b}',
        '\u{61d}',
        '\u{180a}',
        '\u{1810}',
        '\u{200a}',
        '\u{2010}',
        '\u{2027}',
        '\u{202f}',
        '\u{205f}',
        '\u{2070}',
        '\u{fdff}',
        '\u{fe10}',
        '\u{fefe}',
        '\u{ff00}',
        '\u{fff8}',
        '\u{fffc}',
        '\u{e0080}',
        '\u{e00ff}',
        '\u{e01f0}',
        'é',
        '日',
        '😀',
        'ß',
    ];

    #[test]
    fn every_hidden_and_control_character_is_shown_as_its_code() {
        for &c in ESCAPED {
            assert!(is_hidden(c) || c.is_control(), "{:x}", c as u32);
            assert_eq!(visible(&c.to_string()), shown(c), "{:x}", c as u32);
        }
    }

    #[test]
    fn visible_characters_next_to_the_ranges_are_left_alone() {
        for &c in KEPT {
            assert!(!is_hidden(c), "{:x}", c as u32);
            assert_eq!(visible(&c.to_string()), c.to_string(), "{:x}", c as u32);
        }
        assert_eq!(visible("a\nb\tc"), "a\nb\tc");
        assert_eq!(visible("héllo wörld 日本語 😀"), "héllo wörld 日本語 😀");
        assert_eq!(visible(""), "");
    }

    #[test]
    fn a_carriage_return_is_shown_and_never_normalised() {
        assert_eq!(visible("a\r\nb"), "a\\u{d}\nb");
        assert_eq!(visible("a\rb"), "a\\u{d}b");
    }

    #[test]
    fn emoji_joiners_and_selectors_are_shown() {
        assert_eq!(visible("👨\u{200d}👩"), "👨\\u{200d}👩");
        assert_eq!(visible("❤\u{fe0f}"), "❤\\u{fe0f}");
    }

    #[test]
    fn an_ordinary_backslash_is_left_alone() {
        for text in [
            r"grep '\.nix'",
            r"echo a\nb",
            r"C:\Users\me",
            r"\",
            r"\\",
            r"a\ b",
            r"\u",
            r"\ux",
            r"\u 0041",
            r"x\{",
            r"\\\",
        ] {
            assert_eq!(visible(text), text);
        }
    }

    #[test]
    fn typed_escape_text_differs_from_a_real_escape() {
        let real = visible("\u{202e}");
        let typed = visible(r"\u{202e}");
        assert_eq!(real, r"\u{202e}");
        assert_eq!(typed, r"\\u{202e}");
        assert_ne!(real, typed);
    }

    #[test]
    fn runs_of_backslashes_before_an_escape_or_the_text_u_brace() {
        // A run is doubled when what follows could pass for an escape.
        assert_eq!(visible(r"\u{"), r"\\u{");
        assert_eq!(visible(r"\\u{"), r"\\\\u{");
        assert_eq!(visible(r"\\\u{"), r"\\\\\\u{");
        assert_eq!(visible("\\\u{202e}"), "\\\\\\u{202e}");
        assert_eq!(visible("\\\\\u{202e}"), "\\\\\\\\\\u{202e}");
        // The two readings of the same-looking text stay apart.
        assert_ne!(visible("\\\u{202e}"), visible(r"\\u{202e}"));
        assert_ne!(visible("\\\\\u{202e}"), visible(r"\\\u{202e}"));
        // Not followed by such text: untouched.
        assert_eq!(visible("\\\\a\u{202e}"), "\\\\a\\u{202e}");
    }

    #[test]
    fn what_is_shown_decodes_to_exactly_the_input() {
        let samples = [
            r"\u{202e}",
            r"\\u{202e}",
            r"\\\u{202e}",
            "\\\u{202e}",
            "\\\\\u{202e}",
            "\\\u{202e}\\u{202e}",
            "a\u{202e}\\u{202e}\\",
            r"\u{",
            r"u{\u{",
            "line\r\nnext\t\\ \u{0}",
        ];
        for s in samples {
            assert_eq!(unescape(&visible(s)), s, "{s:?} shown as {:?}", visible(s));
        }
    }

    /// The source of `conversation.rs` without its tests.
    fn conversation_code() -> &'static str {
        let source = include_str!("conversation.rs");
        &source[..source.find("#[cfg(test)]").expect("tests")]
    }

    fn body<'a>(code: &'a str, start: &str) -> &'a str {
        let from = code.find(start).expect(start);
        let rest = &code[from..];
        &rest[..rest.find("\n    }\n").expect(start)]
    }

    #[test]
    fn cards_and_tool_cards_are_built_only_from_visible_text() {
        let code = conversation_code();
        // One place builds an approval card, and it escapes every field.
        assert_eq!(code.matches("ApprovalCard::new(").count(), 1);
        let new = body(code, "fn new(request: PermissionRequest)");
        assert!(new.contains(".escaped()"), "{new}");
        assert!(
            new.contains("let title = visible(&request.title);"),
            "{new}"
        );
        // One place builds a tool card, after making its text visible.
        assert_eq!(code.matches("Item::Tool(ToolCard {").count(), 1);
        let tool = body(code, "fn tool(&mut self, call: ToolCall)");
        for field in ["title", "kind", "status"] {
            assert!(
                tool.contains(&format!("{field}: call.{field}.as_deref().map(visible)")),
                "{field}"
            );
        }
    }

    #[test]
    fn every_short_text_from_a_tricky_alphabet_decodes_back() {
        let alphabet = ['\\', 'u', '{', '}', '\u{202e}', 'a'];
        let mut count = 0;
        let mut stack: Vec<Vec<char>> = vec![Vec::new()];
        while let Some(text) = stack.pop() {
            let s: String = text.iter().collect();
            assert_eq!(unescape(&visible(&s)), s, "{s:?}");
            count += 1;
            if text.len() < 7 {
                for &c in &alphabet {
                    let mut next = text.clone();
                    next.push(c);
                    stack.push(next);
                }
            }
        }
        assert!(count > 300_000, "{count}");
    }
}
