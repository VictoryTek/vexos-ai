//! Text that must be shown as it is. Agent text on an approval card (the
//! command, a path, a diff) is shown to the user so they can decide, so what
//! they read has to be what is there: a control or invisible character is shown
//! as `\u{hex}`, and nothing is stripped, normalised or hidden.
//!
//! On an approval card (`visible_card`) look-alike spaces such as U+00A0 are
//! shown as `\u{hex}` too; the log sanitizer and tool cards keep them.
//!
//! A real escape and the same characters typed as text must never look alike.
//! A run of backslashes is written twice over when what follows it could be
//! mistaken for an escape: the text `u{` (the start of one) or a character that
//! is escaped here (whose escape starts with one more backslash). Any other
//! backslash is left alone, so `grep '\.nix'` reads as typed. Every output
//! decodes to exactly one input.

/// Characters that hide, reorder or break text on screen without drawing
/// anything, or that look like a space but are not one: soft hyphen, combining
/// grapheme joiner, bidi marks, joiners and zero-width spaces, line and
/// paragraph separators, word joiner and the invisible operators, deprecated
/// format controls, variation selectors, the byte order mark, interlinear
/// annotation marks, tag characters, the four Hangul fillers (which look like a
/// space but do not split words) and the other default-ignorable characters
/// that GTK and the Shell draw as nothing. Ordinary Hangul and CJK letters are
/// not in it.
pub(crate) fn is_hidden(c: char) -> bool {
    matches!(c,
        '\u{AD}' | '\u{34F}' | '\u{61C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}'..='\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}'
        | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
        | '\u{3164}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{FFF0}'..='\u{FFFB}' | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0000}'..='\u{E0FFF}')
}

/// Characters that draw as an ordinary space but are not one, so a shell does
/// not split words on them: no-break space, Ogham space mark, the en and em
/// spaces and the rest of U+2000-200A, narrow no-break space, medium
/// mathematical space and ideographic space. They are escaped on approval
/// cards only (see [`visible_card`]): the log sanitizer and tool cards keep
/// them, and Claude's own reset text uses U+202F.
pub(crate) fn is_lookalike_space(c: char) -> bool {
    matches!(
        c,
        '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// A character shown as its code: a control other than newline and tab, or a
/// hidden one.
fn needs_escape(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t') || is_hidden(c)
}

/// [`needs_escape`], and on an approval card also a look-alike space.
fn needs_escape_on_card(c: char) -> bool {
    needs_escape(c) || is_lookalike_space(c)
}

/// `text` with every control (except newline and tab) and hidden character
/// shown as `\u{hex}`.
pub fn visible(text: &str) -> String {
    render(text, needs_escape)
}

/// [`visible`] for text on an approval card: look-alike spaces are shown as
/// `\u{hex}` too, so `cp a<U+00A0>b /dst` does not read as two words.
pub fn visible_card(text: &str) -> String {
    render(text, needs_escape_on_card)
}

fn render(text: &str, needs_escape: fn(char) -> bool) -> String {
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
        // look-alike spaces and more default-ignorable characters (E7)
        '\u{115f}',
        '\u{1160}',
        '\u{3164}',
        '\u{ffa0}',
        '\u{17b4}',
        '\u{17b5}',
        '\u{1bca0}',
        '\u{1bca3}',
        '\u{1d173}',
        '\u{1d17a}',
        '\u{fff0}',
        '\u{fff8}',
        '\u{e0080}',
        '\u{e00ff}',
        '\u{e01f0}',
        '\u{e0fff}',
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
        '\u{fffc}',
        '\u{17b3}',
        '\u{17b6}',
        '\u{1bc9f}',
        '\u{1bca4}',
        '\u{1d172}',
        '\u{1d17b}',
        '\u{ffef}',
        '\u{e1000}',
        '\u{115e}',
        '\u{1161}',
        '\u{3163}',
        '\u{3165}',
        '\u{ff9f}',
        '\u{ffa1}',
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
    fn a_look_alike_space_is_shown_so_a_command_reads_as_it_runs() {
        // The gap between a and b looks like a space but is a Hangul filler.
        assert_eq!(visible("cp a\u{3164}b /dst"), r"cp a\u{3164}b /dst");
        assert_eq!(
            visible("a\u{115f}b\u{1160}c\u{ffa0}d"),
            r"a\u{115f}b\u{1160}c\u{ffa0}d"
        );
    }

    #[test]
    fn ordinary_hangul_and_cjk_letters_are_left_alone() {
        for text in [
            "한국어",
            "안녕하세요 세계",
            "\u{1100}\u{1161}\u{11a8}",
            "ㄱㅏㅣ ㅥ",
            "\u{ffa1}\u{ffa2}\u{ffc2}",
            "日本語 中文 漢字",
            "ひらがな カタカナ ｶﾀｶﾅ",
            "mixed 한글 and 中文 and 日本語 text",
        ] {
            assert_eq!(visible(text), text, "{text}");
        }
        // every Hangul syllable and CJK ideograph, and every jamo, compatibility
        // jamo and halfwidth jamo except the four fillers, is not hidden
        let fillers = ['\u{115f}', '\u{1160}', '\u{3164}', '\u{ffa0}'];
        let letters = ('\u{ac00}'..='\u{d7a3}')
            .chain('\u{4e00}'..='\u{9fff}')
            .chain('\u{3400}'..='\u{4dbf}')
            .chain('\u{3040}'..='\u{30ff}')
            .chain('\u{1100}'..='\u{11ff}')
            .chain('\u{3131}'..='\u{318e}')
            .chain('\u{ffa1}'..='\u{ffdc}');
        for c in letters {
            if fillers.contains(&c) {
                continue;
            }
            assert!(!is_hidden(c), "{:x}", c as u32);
        }
        for c in fillers {
            assert!(is_hidden(c), "{:x}", c as u32);
        }
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

    /// Look-alike spaces (E9): every one, and the ends of each range.
    const SPACES: &[char] = &[
        '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2005}', '\u{2009}', '\u{200a}',
        '\u{202f}', '\u{205f}', '\u{3000}',
    ];

    /// Next to them: other visible characters, left alone on a card too.
    const NEAR_SPACES: &[char] = &[
        '\u{20}', '\u{a1}', '\u{167f}', '\u{1681}', '\u{1fff}', '\u{2030}', '\u{205e}', '\u{2fff}',
        '\u{3001}', '\u{2800}', 'a', '日',
    ];

    #[test]
    fn a_card_shows_every_look_alike_space_as_its_code() {
        for &c in SPACES {
            assert_eq!(visible_card(&c.to_string()), shown(c), "{:x}", c as u32);
            assert_eq!(
                visible_card(&format!("cp a{c}b /dst")),
                format!("cp a{}b /dst", shown(c))
            );
        }
    }

    #[test]
    fn a_card_leaves_the_characters_next_to_the_spaces_alone() {
        for &c in NEAR_SPACES {
            assert_eq!(
                visible_card(&c.to_string()),
                c.to_string(),
                "{:x}",
                c as u32
            );
        }
        assert_eq!(visible_card("a b\tc\nd"), "a b\tc\nd");
    }

    #[test]
    fn the_log_and_tool_text_still_keep_look_alike_spaces() {
        // Only cards escape them: the log sanitizer and tool cards do not, and
        // U+202F is in Claude's own reset text.
        for &c in SPACES {
            assert!(!is_hidden(c), "{:x}", c as u32);
            assert_eq!(visible(&c.to_string()), c.to_string(), "{:x}", c as u32);
        }
    }

    #[test]
    fn a_card_escapes_everything_visible_does_and_more() {
        for &c in ESCAPED {
            assert_eq!(visible_card(&c.to_string()), visible(&c.to_string()));
        }
        for &c in KEPT {
            if SPACES.contains(&c) {
                continue;
            }
            assert_eq!(
                visible_card(&c.to_string()),
                c.to_string(),
                "{:x}",
                c as u32
            );
        }
    }

    #[test]
    fn a_real_look_alike_escape_and_the_same_text_typed_stay_apart() {
        assert_eq!(visible_card("\u{a0}"), r"\u{a0}");
        assert_eq!(visible_card(r"\u{a0}"), r"\\u{a0}");
        assert_eq!(visible_card("\\\u{a0}"), "\\\\\\u{a0}");
        assert_ne!(visible_card("\\\u{a0}"), visible_card(r"\\u{a0}"));
        for s in ["a\u{a0}b", "\\\u{3000}", r"\u{a0}", "\\\u{202f}\\\u{202e}x"] {
            assert_eq!(unescape(&visible_card(s)), s, "{s:?}");
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
            new.contains("let title = visible_card(&request.title);"),
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
