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

/// What a block is, for drawing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Also a quote or a raw HTML block: shown as text.
    Paragraph,
    Heading,
    /// One list item. Its first run starts with the marker (`• ` or `3. `).
    ListItem,
    CodeBlock,
    /// A table as monospace text: a header run and the body.
    Table,
    Rule,
}

/// One block of a reply: its kind, its list depth and its runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// How deeply a list item is nested (0 for the first level and for every
    /// other kind of block), at most `MAX_INDENT`.
    pub depth: usize,
    pub runs: Vec<Run>,
}

/// `text` as blocks of literal text, for a front end that draws one widget per
/// block. The same text and styles as [`markdown`], without the blank-line
/// runs between blocks and without a list's indentation (that is `depth`).
pub fn blocks(text: &str) -> Vec<Block> {
    read(text).finish_blocks()
}

/// `text` as runs of literal text. Adjacent runs never share a style.
pub fn markdown(text: &str) -> Vec<Run> {
    read(text).finish()
}

/// Reads `text` once; the runs and the blocks are both built on the way.
fn read(text: &str) -> Out<'_> {
    let mut out = Out {
        source: text,
        ..Out::default()
    };
    for (event, range) in Parser::new_ext(text, Options::ENABLE_TABLES).into_offset_iter() {
        out.event(event, range);
    }
    out
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
    /// The blocks finished so far, and the one being built.
    blocks: Vec<Block>,
    open: Option<Block>,
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

    /// Text of the reply: into the open block and into the flat runs.
    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(cell) = self.table.as_mut().and_then(|t| t.cell.as_mut()) {
            cell.push_str(text);
            return;
        }
        self.push_block(text, style);
        self.push_flat(text, style);
    }

    /// Spacing between blocks: only the flat runs have it.
    fn push_flat(&mut self, text: &str, style: Style) {
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
        self.push_flat(&"\n".repeat(missing), Style::default());
    }

    /// A new line, unless already at the start of one.
    fn line(&mut self) {
        if !self.runs.is_empty() && self.trailing_newlines() == 0 {
            self.push_flat("\n", Style::default());
        }
    }

    /// Right after a list marker, in the item's own block: what starts here
    /// stays in the item. (`after_marker` alone is still set after an item that
    /// had nothing after its marker, when no block is open.)
    fn at_item_start(&self) -> bool {
        self.after_marker
            && self
                .open
                .as_ref()
                .is_some_and(|b| b.kind == BlockKind::ListItem)
    }

    /// Starts a block, finishing the one before it.
    fn begin(&mut self, kind: BlockKind, depth: usize) {
        self.end_block();
        self.open = Some(Block {
            kind,
            depth,
            runs: Vec::new(),
        });
    }

    /// Finishes the open block: no newline at its end, and none if it is empty.
    fn end_block(&mut self) {
        let Some(mut block) = self.open.take() else {
            return;
        };
        while let Some(last) = block.runs.last_mut() {
            let kept = last.text.trim_end_matches('\n').len();
            last.text.truncate(kept);
            if !last.text.is_empty() {
                break;
            }
            block.runs.pop();
        }
        if !block.runs.is_empty() {
            self.blocks.push(block);
        }
    }

    /// Text goes to the open block; with none open, a paragraph starts.
    fn push_block(&mut self, text: &str, style: Style) {
        let block = self.open.get_or_insert_with(|| Block {
            kind: BlockKind::Paragraph,
            depth: 0,
            runs: Vec::new(),
        });
        match block.runs.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => block.runs.push(Run {
                text: text.to_owned(),
                style,
            }),
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
                self.begin(BlockKind::Rule, 0);
                self.push("———", Style::default());
                self.end_block();
            }
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::BlockQuote(_) | Tag::HtmlBlock => {
                // The first paragraph of an item stays in the item's block.
                let in_item = self.at_item_start();
                self.block();
                if !in_item {
                    self.begin(BlockKind::Paragraph, 0);
                }
            }
            Tag::Heading { .. } => {
                // Like a paragraph, one that opens an item stays in the item.
                let in_item = self.at_item_start();
                self.block();
                self.heading = true;
                if !in_item {
                    self.begin(BlockKind::Heading, 0);
                }
            }
            Tag::CodeBlock(_) => {
                let in_item = self.at_item_start();
                self.block();
                self.code_block = true;
                if !in_item {
                    self.begin(BlockKind::CodeBlock, 0);
                }
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
                self.begin(BlockKind::ListItem, depth);
                self.push_flat(&format!("{}{marker}", "  ".repeat(depth)), Style::default());
                self.push_block(&marker, Style::default());
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
                let in_item = self.at_item_start();
                self.block();
                self.table = Some(Table {
                    aligns,
                    ..Table::default()
                });
                if !in_item {
                    self.begin(BlockKind::Table, 0);
                }
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
            // Text after a block, inside the same item, is a new block: so
            // every block ends at its closing tag.
            TagEnd::Paragraph | TagEnd::BlockQuote(_) | TagEnd::HtmlBlock | TagEnd::Item => {
                self.end_block();
            }
            TagEnd::Heading(_) => {
                self.heading = false;
                self.end_block();
            }
            TagEnd::CodeBlock => {
                self.code_block = false;
                self.end_block();
            }
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
            TagEnd::Table => {
                self.show_table();
                self.end_block();
            }
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

    fn finish_blocks(mut self) -> Vec<Block> {
        self.show_table();
        self.end_block();
        self.blocks
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

    // Blocks.

    fn block(kind: BlockKind, depth: usize, runs: Vec<Run>) -> Block {
        Block { kind, depth, runs }
    }

    fn para(runs: Vec<Run>) -> Block {
        block(BlockKind::Paragraph, 0, runs)
    }

    fn item(depth: usize, runs: Vec<Run>) -> Block {
        block(BlockKind::ListItem, depth, runs)
    }

    #[test]
    fn a_paragraph_keeps_its_styled_runs_and_soft_breaks() {
        assert_eq!(
            blocks("Hello **bold** and *it*"),
            [para(vec![
                run("Hello ", PLAIN),
                run("bold", BOLD),
                run(" and ", PLAIN),
                run("it", ITALIC)
            ])]
        );
        assert_eq!(blocks("a\nb"), [para(vec![run("a\nb", PLAIN)])]);
        assert_eq!(
            blocks("a\n\nb"),
            [para(vec![run("a", PLAIN)]), para(vec![run("b", PLAIN)])]
        );
    }

    #[test]
    fn a_heading_is_its_own_block() {
        assert_eq!(
            blocks("# Title\n\ntext"),
            [
                block(BlockKind::Heading, 0, vec![run("Title", HEADING)]),
                para(vec![run("text", PLAIN)])
            ]
        );
    }

    #[test]
    fn list_items_are_blocks_with_their_marker_and_depth() {
        assert_eq!(
            blocks("- one\n- two"),
            [
                item(0, vec![run("• one", PLAIN)]),
                item(0, vec![run("• two", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("1. a\n2. b"),
            [
                item(0, vec![run("1. a", PLAIN)]),
                item(0, vec![run("2. b", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("- a\n  - b\n    - c\n- d"),
            [
                item(0, vec![run("• a", PLAIN)]),
                item(1, vec![run("• b", PLAIN)]),
                item(2, vec![run("• c", PLAIN)]),
                item(0, vec![run("• d", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("- [x] done\n- [ ] todo"),
            [
                item(0, vec![run("• [x] done", PLAIN)]),
                item(0, vec![run("• [ ] todo", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("- **b** rest"),
            [item(
                0,
                vec![run("• ", PLAIN), run("b", BOLD), run(" rest", PLAIN)]
            )]
        );
    }

    #[test]
    fn a_second_paragraph_in_an_item_is_its_own_block() {
        assert_eq!(
            blocks("- a\n\n  second\n- b"),
            [
                item(0, vec![run("• a", PLAIN)]),
                para(vec![run("second", PLAIN)]),
                item(0, vec![run("• b", PLAIN)])
            ]
        );
    }

    #[test]
    fn code_blocks_hold_their_text_without_the_final_newline() {
        let code = |text: &str| block(BlockKind::CodeBlock, 0, vec![run(text, CODE)]);
        assert_eq!(blocks("```\nlet x = 1;\n```"), [code("let x = 1;")]);
        assert_eq!(blocks("```rust\na\n\nb\n```"), [code("a\n\nb")]);
        assert_eq!(blocks("    indented\n    more"), [code("indented\nmore")]);
        assert_eq!(
            blocks("before\n\n```\nx\n```\n\nafter"),
            [
                para(vec![run("before", PLAIN)]),
                code("x"),
                para(vec![run("after", PLAIN)])
            ]
        );
    }

    #[test]
    fn text_after_a_code_block_or_heading_inside_an_item_is_a_new_block() {
        let code = |text: &str| block(BlockKind::CodeBlock, 0, vec![run(text, CODE)]);
        assert_eq!(
            blocks(
                "1. Run this:\n   ```\n   just build\n   ```\n   Then check the output.\n2. Next"
            ),
            [
                item(0, vec![run("1. Run this:", PLAIN)]),
                code("just build"),
                para(vec![run("Then check the output.", PLAIN)]),
                item(0, vec![run("2. Next", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("- a\n  # Title\n  body text"),
            [
                item(0, vec![run("• a", PLAIN)]),
                block(BlockKind::Heading, 0, vec![run("Title", HEADING)]),
                para(vec![run("body text", PLAIN)])
            ]
        );
    }

    #[test]
    fn an_item_that_starts_with_a_heading_or_code_keeps_its_bullet_with_it() {
        assert_eq!(
            blocks("- # Title\n  body text"),
            [
                item(0, vec![run("• ", PLAIN), run("Title", HEADING)]),
                para(vec![run("body text", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("- ```\n  code\n  ```\n- next"),
            [
                item(0, vec![run("• ", PLAIN), run("code", CODE)]),
                item(0, vec![run("• next", PLAIN)])
            ]
        );
    }

    #[test]
    fn a_block_after_an_empty_item_keeps_its_own_kind() {
        let code = |text: &str| block(BlockKind::CodeBlock, 0, vec![run(text, CODE)]);
        assert_eq!(
            blocks("-\n# h"),
            [
                item(0, vec![run("• ", PLAIN)]),
                block(BlockKind::Heading, 0, vec![run("h", HEADING)])
            ]
        );
        assert_eq!(
            blocks("1.\n```bash\nnix build\n```"),
            [item(0, vec![run("1. ", PLAIN)]), code("nix build")]
        );
        // Text after an empty item is a paragraph of its own, also inside a list.
        assert_eq!(
            blocks("-\ntext"),
            [
                item(0, vec![run("• ", PLAIN)]),
                para(vec![run("text", PLAIN)])
            ]
        );
        assert_eq!(
            blocks("- a\n\n  -\n  tail"),
            [
                item(0, vec![run("• a", PLAIN)]),
                item(1, vec![run("• ", PLAIN)]),
                para(vec![run("tail", PLAIN)])
            ]
        );
    }

    #[test]
    fn an_item_that_starts_with_a_table_keeps_its_bullet_with_it() {
        let table = "| a | b |\n|---|---|\n| 1 | 2 |";
        let got = blocks(&format!("- {}", table.replace('\n', "\n  ")));
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].kind, BlockKind::ListItem);
        assert_eq!(got[0].runs[0], run("• ", PLAIN));
        assert_eq!(got[0].runs[1..], markdown(table)[..]);
    }

    #[test]
    fn a_table_is_one_block_with_the_runs_the_flat_output_has() {
        let table = "| a | b |\n|---|---|\n| 1 | 22 |";
        assert_eq!(blocks(table), [block(BlockKind::Table, 0, markdown(table))]);
        let around = format!("intro\n\n{table}\n\noutro");
        let got = blocks(&around);
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].kind, BlockKind::Table);
        assert_eq!(got[1].runs, markdown(table));
    }

    #[test]
    fn a_rule_a_quote_and_raw_html_are_blocks_of_text() {
        assert_eq!(
            blocks("a\n\n---\n\nb"),
            [
                para(vec![run("a", PLAIN)]),
                block(BlockKind::Rule, 0, vec![run("———", PLAIN)]),
                para(vec![run("b", PLAIN)])
            ]
        );
        assert_eq!(blocks("> quote"), [para(vec![run("quote", PLAIN)])]);
        assert_eq!(
            blocks("<script>alert(1)</script>\n\n**x** <b>y</b>"),
            [
                para(vec![run("<script>alert(1)</script>", PLAIN)]),
                para(vec![run("x", BOLD), run(" <b>y</b>", PLAIN)])
            ]
        );
    }

    #[test]
    fn links_and_images_are_text_followed_by_their_target() {
        let b = blocks("[t](http://x) ![alt](http://y)");
        assert_eq!(b.len(), 1);
        let text: String = b[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "t (http://x) [image: alt] (http://y)");
        let flat: String = markdown("[t](http://x) ![alt](http://y)")
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(text, flat);
    }

    #[test]
    fn nothing_gives_no_blocks() {
        assert!(blocks("").is_empty());
        assert!(blocks("   \n\n  \n").is_empty());
    }

    const CORPUS: &[&str] = &[
        "plain",
        "a\n\nb\n\nc",
        "# H1\n## H2\ntext **b** *i* `c`",
        "- a\n- b\n  - c\n    - d\n- e",
        "1. one\n2. two\n   - nested\n3. three",
        "- a\n\n  second para\n\n  ```\n  code\n  ```\n- b",
        "```\ncode\n```\n\n    indented\n\ntext",
        "| a | b |\n|:--|--:|\n| 1 | 2 |\n| 3 |",
        "text\n\n| h |\n|---|\n| x |\n\nmore",
        "> quote\n> more\n\n> second",
        "---\n\ntext\n\n***",
        "[l](http://a) ![i](http://b) <http://c> <b>html</b>",
        "- [x] done\n- [ ] todo\n- plain",
        "<div>\nraw\n</div>\n\nafter",
        "**bold *both* bold** and _it_ and ~~del~~",
        "line one  \nline two\\\nline three",
        "```\nunterminated",
        "| unterminated | table",
        "a\n- b\n- c\nd",
        "text with \u{202e} and \u{0} controls\n\n- \u{1b}[31mred",
    ];

    /// The text and the styles, without spacing: what a front end draws.
    fn essence(runs: &[Run]) -> (String, [String; 4]) {
        let mut styles: [String; 4] = Default::default();
        let mut all = String::new();
        for run in runs {
            let text: String = run.text.chars().filter(|c| !c.is_whitespace()).collect();
            all.push_str(&text);
            for (slot, on) in styles.iter_mut().zip([
                run.style.bold,
                run.style.italic,
                run.style.code,
                run.style.heading,
            ]) {
                if on {
                    slot.push_str(&text);
                }
            }
        }
        (all, styles)
    }

    #[test]
    fn the_blocks_carry_the_same_text_and_styles_as_the_flat_runs() {
        for text in CORPUS {
            let flat = markdown(text);
            let all: Vec<Run> = blocks(text).into_iter().flat_map(|b| b.runs).collect();
            assert_eq!(essence(&all), essence(&flat), "{text:?}");
        }
    }

    #[test]
    fn blocks_have_no_empty_parts_and_no_blank_edges() {
        for text in CORPUS {
            for b in blocks(text) {
                assert!(!b.runs.is_empty(), "{text:?}");
                assert!(b.runs.iter().all(|r| !r.text.is_empty()), "{text:?}");
                assert!(!b.runs[0].text.starts_with('\n'), "{text:?} {b:?}");
                assert!(
                    !b.runs.last().unwrap().text.ends_with('\n'),
                    "{text:?} {b:?}"
                );
                assert!(
                    b.runs.windows(2).all(|w| w[0].style != w[1].style),
                    "{text:?} {b:?}"
                );
                assert!(b.depth <= MAX_INDENT);
                if b.kind != BlockKind::ListItem {
                    assert_eq!(b.depth, 0, "{text:?}");
                }
            }
        }
    }

    #[test]
    fn deep_lists_stop_at_the_depth_cap_and_large_input_does_not_panic() {
        let deep: String = (0..60)
            .map(|i| format!("{}- x\n", "  ".repeat(i)))
            .collect();
        let b = blocks(&deep);
        assert!(b.iter().all(|x| x.depth <= MAX_INDENT));
        assert!(b.iter().any(|x| x.depth == MAX_INDENT));
        let big = "- item **x** `y`\n\n".repeat(20_000);
        assert!(blocks(&big).len() >= 20_000);
    }
}
