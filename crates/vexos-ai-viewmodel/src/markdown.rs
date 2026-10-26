//! An agent reply as formatted text. Agent output is untrusted, so this only
//! ever produces literal text runs, each with a style. The window inserts the
//! text as is, with a text tag per style, and never parses markup, so nothing
//! in a reply can become markup. Raw HTML is shown as text. Links and images
//! are text followed by their real target: never clickable, never loaded.
//!
//! Only agent replies go through here. Approval cards (command, plan, diff)
//! and the user's own messages are shown literally.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

/// Lists nested deeper than this are indented as this level.
const MAX_INDENT: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    /// Inline code or a code block: monospace.
    pub code: bool,
    pub heading: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

/// `text` as runs of literal text. Adjacent runs never share a style.
pub fn markdown(text: &str) -> Vec<Run> {
    let mut out = Out::default();
    for event in Parser::new_ext(text, Options::empty()) {
        out.event(event);
    }
    out.finish()
}

#[derive(Default)]
struct Out {
    runs: Vec<Run>,
    bold: usize,
    italic: usize,
    heading: bool,
    code_block: bool,
    /// The next number of each open list; `None` for a bulleted one.
    lists: Vec<Option<u64>>,
    /// The targets of the open links and images, shown after their text.
    targets: Vec<Option<String>>,
    /// Just after a list marker: the item's first block stays on its line.
    after_marker: bool,
}

impl Out {
    fn style(&self) -> Style {
        Style {
            bold: self.bold > 0 || self.heading,
            italic: self.italic > 0,
            code: self.code_block,
            heading: self.heading,
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.after_marker = false;
        match self.runs.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => self.runs.push(Run {
                text: text.to_owned(),
                style,
            }),
        }
    }

    fn text(&mut self, text: &str) {
        self.push(text, self.style());
    }

    fn trailing_newlines(&self) -> usize {
        let mut count = 0;
        for run in self.runs.iter().rev() {
            let n = run.text.chars().rev().take_while(|&c| c == '\n').count();
            count += n;
            if n < run.text.chars().count() || count >= 2 {
                break;
            }
        }
        count
    }

    /// A new block: a blank line after whatever came before.
    fn block(&mut self) {
        if self.after_marker || self.runs.is_empty() {
            return;
        }
        let missing = 2 - self.trailing_newlines().min(2);
        self.push(&"\n".repeat(missing), Style::default());
    }

    /// A new line, unless already at the start of one.
    fn line(&mut self) {
        if !self.runs.is_empty() && self.trailing_newlines() == 0 {
            self.push("\n", Style::default());
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => self.text(&text),
            Event::Code(text) => self.push(
                &text,
                Style {
                    code: true,
                    ..self.style()
                },
            ),
            Event::SoftBreak | Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.block();
                self.push("———", Style::default());
            }
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::BlockQuote(_) | Tag::HtmlBlock => self.block(),
            Tag::Heading { .. } => {
                self.block();
                self.heading = true;
            }
            Tag::CodeBlock(_) => {
                self.block();
                self.code_block = true;
            }
            Tag::List(first) => {
                if self.lists.is_empty() {
                    self.block();
                }
                self.lists.push(first);
            }
            Tag::Item => {
                self.line();
                let depth = self.lists.len().saturating_sub(1).min(MAX_INDENT);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".to_owned(),
                };
                self.push(&format!("{}{marker}", "  ".repeat(depth)), Style::default());
                self.after_marker = true;
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                // An autolink's text is its target already.
                let shown = !matches!(link_type, LinkType::Autolink | LinkType::Email);
                self.targets.push(shown.then(|| dest_url.into_string()));
            }
            Tag::Image { dest_url, .. } => {
                self.text("[image: ");
                self.targets.push(Some(dest_url.into_string()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => self.heading = false,
            TagEnd::CodeBlock => self.code_block = false,
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Image => {
                self.text("]");
                self.show_target();
            }
            TagEnd::Link => self.show_target(),
            _ => {}
        }
    }

    fn show_target(&mut self) {
        if let Some(Some(target)) = self.targets.pop() {
            self.push(&format!(" ({target})"), Style::default());
        }
    }

    fn finish(mut self) -> Vec<Run> {
        while let Some(last) = self.runs.last_mut() {
            let kept = last.text.trim_end_matches('\n').len();
            last.text.truncate(kept);
            if !last.text.is_empty() {
                break;
            }
            self.runs.pop();
        }
        self.runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Style = Style {
        bold: false,
        italic: false,
        code: false,
        heading: false,
    };
    const BOLD: Style = Style {
        bold: true,
        ..PLAIN
    };
    const ITALIC: Style = Style {
        italic: true,
        ..PLAIN
    };
    const CODE: Style = Style {
        code: true,
        ..PLAIN
    };
    const HEADING: Style = Style {
        bold: true,
        heading: true,
        ..PLAIN
    };

    fn run(text: &str, style: Style) -> Run {
        Run {
            text: text.into(),
            style,
        }
    }

    fn text_of(runs: &[Run]) -> String {
        runs.iter().map(|r| r.text.as_str()).collect()
    }

    /// Everything shown, as one plain string, with no style anywhere.
    fn assert_plain(input: &str, shown: &str) {
        assert_eq!(markdown(input), [run(shown, PLAIN)], "{input}");
    }

    // Formatting -------------------------------------------------------------

    #[test]
    fn bold_italic_and_inline_code() {
        assert_eq!(
            markdown("Run **this** _now_ with `ls -l`."),
            [
                run("Run ", PLAIN),
                run("this", BOLD),
                run(" ", PLAIN),
                run("now", ITALIC),
                run(" with ", PLAIN),
                run("ls -l", CODE),
                run(".", PLAIN),
            ]
        );
    }

    #[test]
    fn paragraphs_and_headings_are_separated_by_a_blank_line() {
        assert_eq!(
            markdown("# Swap\n\nIt is off.\nCheck it.\n\nDone."),
            [
                run("Swap", HEADING),
                run("\n\nIt is off.\nCheck it.\n\nDone.", PLAIN),
            ]
        );
    }

    #[test]
    fn a_fenced_code_block_is_literal_monospace() {
        assert_eq!(
            markdown("Try:\n\n```sh\nls **not bold** <b>x</b>\n```\n\nThen."),
            [
                run("Try:\n\n", PLAIN),
                run("ls **not bold** <b>x</b>\n", CODE),
                run("\nThen.", PLAIN),
            ]
        );
    }

    #[test]
    fn lists_get_bullets_numbers_and_indents() {
        assert_plain(
            "- one\n- two\n  1. a\n  2. b\n- three",
            "• one\n• two\n  1. a\n  2. b\n• three",
        );
        assert_plain("3. c\n4. d", "3. c\n4. d");
    }

    #[test]
    fn a_loose_list_keeps_each_item_on_its_marker_line() {
        assert_plain("- one\n\n- two", "• one\n• two");
    }

    // Untrusted input ----------------------------------------------------------

    #[test]
    fn html_is_shown_as_literal_text() {
        assert_plain("<script>alert(1)</script>", "<script>alert(1)</script>");
        assert_plain(
            "a <b>bold</b> <img src=x onerror=alert(1)>",
            "a <b>bold</b> <img src=x onerror=alert(1)>",
        );
    }

    #[test]
    fn pango_markup_is_shown_as_literal_text() {
        let input = r#"<span foreground="red" size="xx-large">big</span>"#;
        assert_plain(input, input);
        // Entities decode to the characters, still as text.
        assert_plain(
            "&lt;span foreground=&quot;red&quot;&gt;x&lt;/span&gt; &amp;amp;",
            r#"<span foreground="red">x</span> &amp;"#,
        );
    }

    #[test]
    fn markup_inside_formatting_stays_text() {
        assert_eq!(
            markdown("**<span weight=\"bold\">x</span>**"),
            [run("<span weight=\"bold\">x</span>", BOLD)]
        );
    }

    #[test]
    fn links_are_text_with_their_real_target() {
        assert_plain(
            "[click](javascript:alert(1))",
            "click (javascript:alert(1))",
        );
        assert_plain(
            "[https://nixos.org](https://evil.example/x)",
            "https://nixos.org (https://evil.example/x)",
        );
        assert_plain("<https://nixos.org>", "https://nixos.org");
        assert_plain(
            "[ref]\n\n[ref]: file:///etc/shadow",
            "ref (file:///etc/shadow)",
        );
    }

    #[test]
    fn images_are_never_loaded_only_named() {
        assert_plain(
            "![logo](https://tracker.example/p.png)",
            "[image: logo] (https://tracker.example/p.png)",
        );
    }

    #[test]
    fn a_very_long_line_is_kept_whole() {
        let line = "x".repeat(200_000);
        assert_plain(&line, &line);
    }

    #[test]
    fn an_unterminated_code_fence_runs_to_the_end_as_code() {
        assert_eq!(
            markdown("Run:\n```\nrm -rf /tmp/x\n**not bold**"),
            [
                run("Run:\n\n", PLAIN),
                run("rm -rf /tmp/x\n**not bold**", CODE),
            ]
        );
    }

    #[test]
    fn deeply_nested_lists_render_with_a_capped_indent() {
        let depth = 300;
        let input: String = (0..depth)
            .map(|level| format!("{}- item{level}\n", "  ".repeat(level)))
            .collect();
        let shown = text_of(&markdown(&input));
        for level in 0..depth {
            assert!(shown.contains(&format!("• item{level}")), "item{level}");
        }
        let widest = shown.lines().map(|l| l.len() - l.trim_start().len()).max();
        assert_eq!(widest, Some(2 * MAX_INDENT));
    }

    #[test]
    fn nothing_is_lost_from_what_the_agent_wrote() {
        for input in ["a\\*b", "$x$", "[^1]", "- [x] done", "| a | b |"] {
            assert!(!text_of(&markdown(input)).is_empty(), "{input}");
        }
        assert_eq!(markdown(""), []);
    }
}
