//! An agent reply as formatted text. Agent output is untrusted, so this only
//! ever produces literal text runs, each with a style. The window inserts the
//! text as is, with a text tag per style, and never parses markup, so nothing
//! in a reply can become markup. Raw HTML is shown as text. Links and images
//! are text followed by their real target: never clickable, never loaded.
//!
//! Tables are monospace text: each cell's content as plain text, padded into
//! columns, the header row bold with a rule under it. Formatting inside a
//! cell is dropped, link and image targets are kept, and cells past the
//! header's count are kept as one more cell. Widths are counted in
//! characters, so wide characters (CJK, emoji) and tabs can misalign a
//! column; the text itself is unaffected.
//!
//! Only agent replies go through here. Approval cards (command, plan, diff)
//! and the user's own messages are shown literally.

use std::ops::Range;

use pulldown_cmark::{Alignment, Event, LinkType, Options, Parser, Tag, TagEnd};

/// Lists nested deeper than this are indented as this level.
const MAX_INDENT: usize = 8;
/// Columns are padded to at most this many characters; a longer cell is kept
/// whole and pushes the rest of its line along.
const MAX_COLUMN: usize = 40;
/// A table whose padded text would be longer than this is shown unaligned, so
/// a short reply can't become a huge one.
const MAX_PADDED: usize = 100_000;
const SEPARATOR: &str = " │ ";

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
    let mut out = Out {
        source: text,
        ..Out::default()
    };
    for (event, range) in Parser::new_ext(text, Options::ENABLE_TABLES).into_offset_iter() {
        out.event(event, range);
    }
    out.finish()
}

const TABLE: Style = Style {
    bold: false,
    italic: false,
    code: true,
    heading: false,
};
const TABLE_HEADER: Style = Style {
    bold: true,
    ..TABLE
};

/// A table being read: its cells as plain text, shown when it ends.
#[derive(Default)]
struct Table {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    /// The cell being read; text goes here instead of into runs.
    cell: Option<String>,
    /// Where the current row ends in the source, and where its last cell
    /// ended: anything between them is cells the parser dropped.
    row_end: usize,
    cell_end: usize,
}

impl Table {
    fn end_row(&mut self, source: &str) {
        let dropped = source
            .get(self.cell_end..self.row_end)
            .unwrap_or_default()
            .trim()
            .trim_matches('|')
            .trim();
        let mut row = std::mem::take(&mut self.row);
        if !dropped.is_empty() {
            row.push(dropped.to_owned());
        }
        self.rows.push(row);
    }

    /// The header line, and everything under it.
    fn render(&self) -> (String, String) {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0; columns];
        for row in &self.rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.chars().count().min(MAX_COLUMN));
            }
        }
        let line_width: usize = widths.iter().sum::<usize>() + SEPARATOR.len() * columns;
        let aligned = line_width.saturating_mul(self.rows.len()) <= MAX_PADDED;
        let line = |row: &[String]| {
            let mut cells: Vec<String> = Vec::with_capacity(columns);
            for (i, &width) in widths.iter().enumerate() {
                let cell = row.get(i).map_or("", String::as_str);
                if !aligned {
                    cells.push(cell.to_owned());
                    continue;
                }
                let pad = width.saturating_sub(cell.chars().count());
                cells.push(match self.aligns.get(i) {
                    Some(Alignment::Right) => format!("{}{cell}", " ".repeat(pad)),
                    Some(Alignment::Center) => {
                        format!("{}{cell}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2))
                    }
                    _ => format!("{cell}{}", " ".repeat(pad)),
                });
            }
            if !aligned {
                // Unaligned, empty cells at the end say nothing.
                while cells.last().is_some_and(String::is_empty) {
                    cells.pop();
                }
            }
            cells.join(SEPARATOR).trim_end().to_owned()
        };
        let mut rows = self.rows.iter();
        let header = rows.next().map(|row| line(row)).unwrap_or_default();
        let rule = if aligned {
            widths
                .iter()
                .map(|&w| "─".repeat(w))
                .collect::<Vec<_>>()
                .join("─┼─")
        } else {
            "───".to_owned()
        };
        let mut body = rule;
        for row in rows {
            body.push('\n');
            body.push_str(&line(row));
        }
        (header, body)
    }
}

#[derive(Default)]
struct Out<'a> {
    source: &'a str,
    table: Option<Table>,
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

impl Out<'_> {
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
        if let Some(cell) = self.table.as_mut().and_then(|t| t.cell.as_mut()) {
            cell.push_str(text);
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

    fn event(&mut self, event: Event, range: Range<usize>) {
        if let Some(table) = self.table.as_mut() {
            match &event {
                Event::Start(Tag::TableHead | Tag::TableRow) => {
                    table.row_end = range.end;
                    table.cell_end = range.start;
                }
                Event::End(TagEnd::TableCell) => table.cell_end = range.end,
                _ => {}
            }
        }
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
            Tag::Table(aligns) => {
                self.block();
                self.table = Some(Table {
                    aligns,
                    ..Table::default()
                });
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.cell = Some(String::new());
                }
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
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    let cell = table.cell.take().unwrap_or_default();
                    table.row.push(cell);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                let source = self.source;
                if let Some(table) = self.table.as_mut() {
                    table.end_row(source);
                }
            }
            TagEnd::Table => self.show_table(),
            _ => {}
        }
    }

    fn show_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let (header, body) = table.render();
        self.push(&header, TABLE_HEADER);
        self.push("\n", TABLE);
        self.push(&body, TABLE);
    }

    fn show_target(&mut self) {
        if let Some(Some(target)) = self.targets.pop() {
            self.push(&format!(" ({target})"), Style::default());
        }
    }

    fn finish(mut self) -> Vec<Run> {
        self.show_table();
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

    // Tables -------------------------------------------------------------------

    /// The table as shown: its header line and everything under it.
    fn table(header: &str, body: &str) -> [Run; 2] {
        [run(header, TABLE_HEADER), run(body, TABLE)]
    }

    #[test]
    fn a_table_is_aligned_monospace_with_a_bold_header() {
        assert_eq!(
            markdown("| Breakdown | Tokens |\n|:--|--:|\n| Input | 1.2k |\n| Output | 30 |"),
            table(
                "Breakdown │ Tokens",
                "\n──────────┼───────\nInput     │   1.2k\nOutput    │     30"
            )
        );
        assert_eq!(
            markdown("| a | b | c |\n|:-:|---|--:|\n| wide | x | y |"),
            table(" a   │ b │ c", "\n─────┼───┼──\nwide │ x │ y")
        );
    }

    #[test]
    fn a_table_is_a_block_between_others() {
        let mut expected = vec![run("This session", HEADING), run("\n\n", PLAIN)];
        expected.extend(table("Cost  │ API time", "\n──────┼─────────\n$0.12 │ 4s"));
        expected.push(run("\n\nAfter.", PLAIN));
        assert_eq!(
            markdown(
                "### This session\n\n| Cost | API time |\n|:--|:--|\n| $0.12 | 4s |\n\nAfter."
            ),
            expected
        );
    }

    #[test]
    fn markup_inside_cells_is_literal_text() {
        let input = "| <b>x</b> | **bold** | [l](javascript:x) | `c` |\n|---|---|---|---|\n\
                     | a \\| b | _i_ | ![i](http://t/p.png) | <span foreground=\"red\">r</span> |";
        let rule = ["─".repeat(9), "─".repeat(6), "─".repeat(29), "─".repeat(32)].join("┼");
        assert_eq!(
            markdown(input),
            table(
                "<b>x</b> │ bold │ l (javascript:x)            │ c",
                &format!(
                    "\n{rule}\na | b    │ i    │ [image: i] (http://t/p.png) │ <span foreground=\"red\">r</span>"
                )
            )
        );
    }

    #[test]
    fn ragged_rows_are_padded_and_extra_cells_are_kept() {
        assert_eq!(
            markdown("| a | b |\n|---|---|\n| 1 |\n| 1 | 2 | 3 | **4** |"),
            table("a │ b │", "\n──┼───┼──────────\n1 │   │\n1 │ 2 │ 3 | **4**")
        );
    }

    #[test]
    fn empty_cells_stay_empty() {
        assert_eq!(
            markdown("| a | |\n|---|---|\n| | b |\n| | |"),
            table("a │", "\n──┼──\n  │ b\n  │")
        );
    }

    #[test]
    fn a_very_wide_cell_is_kept_whole_and_columns_stay_capped() {
        let wide = "x".repeat(10_000);
        let shown = text_of(&markdown(&format!(
            "| a | b |\n|---|---|\n| {wide} | 1 |\n| c | 2 |"
        )));
        assert!(shown.contains(&format!("{wide} │ 1")));
        assert!(shown.contains(&format!("c{} │ 2", " ".repeat(MAX_COLUMN - 1))));
    }

    #[test]
    fn many_rows_are_all_shown_aligned() {
        let rows = 5_000;
        let input: String = std::iter::once("| row | n |\n|---|--:|\n".to_owned())
            .chain((0..rows).map(|i| format!("| r{i} | {i} |\n")))
            .collect();
        let shown = text_of(&markdown(&input));
        assert_eq!(shown.lines().count(), rows + 2);
        assert!(shown.contains("\nr0    │    0\n"));
        assert!(shown.ends_with("\nr4999 │ 4999"));
    }

    #[test]
    fn a_table_cannot_blow_up_the_reply() {
        // A wide header and many short rows: padded, this would be megabytes.
        let columns = 1_000;
        let mut input = format!("|{}\n", format!("{}|", "x".repeat(40)).repeat(columns));
        input.push_str(&format!("|{}\n", "---|".repeat(columns)));
        input.push_str(&"| y |\n".repeat(200));
        let shown = text_of(&markdown(&input));
        assert!(
            shown.len() <= 2 * input.len(),
            "{} from {}",
            shown.len(),
            input.len()
        );
        assert_eq!(shown.matches("\ny").count(), 200);
        assert!(shown.starts_with(&format!("{} │ ", "x".repeat(40))));
    }

    #[test]
    fn an_unterminated_table_ends_with_the_input() {
        assert_eq!(markdown("| a |\n|---|"), table("a", "\n─"));
        // A plain line goes on as a row, as in GFM: shown, not lost.
        assert_eq!(
            markdown("| a |\n|---|\n| 1 |\nnot a row"),
            table("a", "\n─────────\n1\nnot a row")
        );
    }

    #[test]
    fn a_pipe_line_without_a_delimiter_row_is_not_a_table() {
        assert_plain("| a | b |\n| 1 | 2 |", "| a | b |\n| 1 | 2 |");
    }
}
