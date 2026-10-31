//! What an approval card shows: the command, the change as a diff, or the plan,
//! read from the request's `rawInput`. Shapes recorded live (spike results,
//! "Plan-exit live check"): Claude `Bash` `{command}`, `Edit`
//! `{file_path, old_string, new_string}`, `MultiEdit` `{file_path, edits}`,
//! `Write` `{file_path, content}`, `ExitPlanMode` `{plan}`. A unified `{filepath,
//! diff}` shape, recorded from the removed OpenCode, is still parsed.

use serde_json::Value;

use crate::escape::visible;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proposal {
    Command(String),
    Diff {
        path: String,
        lines: Vec<DiffLine>,
    },
    /// Empty when the agent sent no plan.
    Plan(String),
    /// Anything else, as pretty-printed JSON.
    Other(String),
    /// Stands in for a request too large to review: only its size, in bytes
    /// after escaping. None of the request is kept.
    TooLarge(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    /// A separator between hunks or edits.
    Hunk(String),
    Context(String),
    Removed(String),
    Added(String),
}

impl Proposal {
    pub fn from_request(kind: Option<&str>, raw: &Value) -> Self {
        let text = |key: &str| raw[key].as_str();
        let path = || {
            ["file_path", "filepath", "filePath"]
                .iter()
                .find_map(|k| text(k))
                .unwrap_or_default()
                .to_owned()
        };
        if let Some(command) = text("command") {
            return Proposal::Command(command.to_owned());
        }
        if kind == Some("switch_mode") || text("plan").is_some() {
            return Proposal::Plan(text("plan").unwrap_or_default().to_owned());
        }
        if let Some(diff) = text("diff") {
            return Proposal::Diff {
                path: path(),
                lines: parse_unified(diff),
            };
        }
        if let (Some(old), Some(new)) = (text("old_string"), text("new_string")) {
            return Proposal::Diff {
                path: path(),
                lines: line_diff(old, new),
            };
        }
        if let Some(edits) = raw["edits"].as_array() {
            let mut lines = Vec::new();
            for (n, edit) in edits.iter().enumerate() {
                lines.push(DiffLine::Hunk(format!("edit {}", n + 1)));
                lines.extend(line_diff(
                    edit["old_string"].as_str().unwrap_or_default(),
                    edit["new_string"].as_str().unwrap_or_default(),
                ));
            }
            return Proposal::Diff {
                path: path(),
                lines,
            };
        }
        if let Some(content) = text("content") {
            return Proposal::Diff {
                path: path(),
                lines: line_diff("", content),
            };
        }
        Proposal::Other(serde_json::to_string_pretty(raw).unwrap_or_default())
    }

    /// How many bytes the proposal shows: its text, and a line break after
    /// every diff line.
    pub fn shown_bytes(&self) -> usize {
        match self {
            Proposal::Command(text) | Proposal::Plan(text) | Proposal::Other(text) => text.len(),
            Proposal::Diff { path, lines } => {
                path.len()
                    + lines
                        .iter()
                        .map(|line| match line {
                            DiffLine::Hunk(text)
                            | DiffLine::Context(text)
                            | DiffLine::Removed(text)
                            | DiffLine::Added(text) => text.len() + 1,
                        })
                        .sum::<usize>()
            }
            Proposal::TooLarge(_) => 0,
        }
    }

    /// The same proposal with every string made visible (see `escape`).
    pub fn escaped(self) -> Proposal {
        match self {
            Proposal::Command(text) => Proposal::Command(visible(&text)),
            Proposal::Plan(text) => Proposal::Plan(visible(&text)),
            Proposal::Other(text) => Proposal::Other(visible(&text)),
            Proposal::TooLarge(bytes) => Proposal::TooLarge(bytes),
            Proposal::Diff { path, lines } => Proposal::Diff {
                path: visible(&path),
                lines: lines
                    .into_iter()
                    .map(|line| match line {
                        DiffLine::Hunk(text) => DiffLine::Hunk(visible(&text)),
                        DiffLine::Context(text) => DiffLine::Context(visible(&text)),
                        DiffLine::Removed(text) => DiffLine::Removed(visible(&text)),
                        DiffLine::Added(text) => DiffLine::Added(visible(&text)),
                    })
                    .collect(),
            },
        }
    }
}

/// The lines of `text`, split on `\n` only, so a `\r` stays in its line and is
/// shown; a final newline adds no empty line.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// Lines of a unified diff, without its file headers.
fn parse_unified(diff: &str) -> Vec<DiffLine> {
    split_lines(diff)
        .into_iter()
        .filter_map(|line| {
            if line.starts_with("Index: ")
                || line.starts_with("===")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
                || line.starts_with('\\')
            {
                None
            } else if line.starts_with("@@") {
                Some(DiffLine::Hunk(line.to_owned()))
            } else if let Some(rest) = line.strip_prefix('+') {
                Some(DiffLine::Added(rest.to_owned()))
            } else if let Some(rest) = line.strip_prefix('-') {
                Some(DiffLine::Removed(rest.to_owned()))
            } else {
                Some(DiffLine::Context(
                    line.strip_prefix(' ').unwrap_or(line).to_owned(),
                ))
            }
        })
        .collect()
}

/// Above this many line pairs, show all old lines removed and all new ones
/// added instead of a minimal diff.
const DIFF_LIMIT: usize = 1_000_000;

/// A line diff of `old` against `new` (longest common subsequence).
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a = split_lines(old);
    let b = split_lines(new);
    let removed = |l: &&str| DiffLine::Removed((*l).to_owned());
    let added = |l: &&str| DiffLine::Added((*l).to_owned());
    if a.len().saturating_mul(b.len()) > DIFF_LIMIT {
        return a.iter().map(removed).chain(b.iter().map(added)).collect();
    }
    // common[i][j]: the longest common subsequence of a[i..] and b[j..].
    let mut common = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            common[i][j] = if a[i] == b[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut lines) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            lines.push(DiffLine::Context(a[i].to_owned()));
            (i, j) = (i + 1, j + 1);
        } else if i < a.len() && (j == b.len() || common[i + 1][j] >= common[i][j + 1]) {
            lines.push(removed(&a[i]));
            i += 1;
        } else {
            lines.push(added(&b[j]));
            j += 1;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use DiffLine::*;
    use serde_json::json;

    fn s(text: &str) -> String {
        text.to_owned()
    }

    #[test]
    fn a_carriage_return_stays_in_a_diff_line() {
        // CRLF to LF is a change the card must show.
        assert_eq!(
            line_diff("a\r\nb", "a\nb"),
            vec![Removed(s("a\r")), Added(s("a")), Context(s("b"))]
        );
        assert_eq!(line_diff("a\r\n", "a\r\n"), vec![Context(s("a\r"))]);
        let raw = json!({ "file_path": "f", "old_string": "a\r\nb", "new_string": "a\nb" });
        let Proposal::Diff { lines, .. } = Proposal::from_request(Some("edit"), &raw).escaped()
        else {
            panic!("not a diff")
        };
        assert_eq!(lines[0], Removed(s("a\\u{d}")));
        let unified = json!({ "filepath": "f", "diff": "@@ -1 +1 @@\r\n-a\r\n+b\r\n" });
        let Proposal::Diff { lines, .. } = Proposal::from_request(None, &unified).escaped() else {
            panic!("not a diff")
        };
        assert_eq!(
            lines,
            vec![
                Hunk(s("@@ -1 +1 @@\\u{d}")),
                Removed(s("a\\u{d}")),
                Added(s("b\\u{d}"))
            ]
        );
    }

    #[test]
    fn lines_are_split_on_newlines_only_and_a_final_newline_adds_none() {
        assert_eq!(line_diff("", ""), vec![]);
        assert_eq!(line_diff("a", "a"), vec![Context(s("a"))]);
        assert_eq!(line_diff("a\n", "a"), vec![Context(s("a"))]);
        assert_eq!(line_diff("\n", ""), vec![Removed(s(""))]);
        assert_eq!(
            line_diff("a\n\n", "a\n"),
            vec![Context(s("a")), Removed(s(""))]
        );
    }

    #[test]
    fn shown_bytes_counts_the_text_and_a_break_after_each_diff_line() {
        assert_eq!(Proposal::Command(s("ls -l")).shown_bytes(), 5);
        assert_eq!(Proposal::Plan(s("é日")).shown_bytes(), 5);
        assert_eq!(Proposal::Other(s("{}")).shown_bytes(), 2);
        assert_eq!(Proposal::TooLarge(99).shown_bytes(), 0);
        assert_eq!(
            Proposal::Diff {
                path: s("/a"),
                lines: vec![
                    Hunk(s("@@")),
                    Added(s("ab")),
                    Removed(s("")),
                    Context(s("é"))
                ]
            }
            .shown_bytes(),
            2 + (2 + 1) + (2 + 1) + 1 + (2 + 1)
        );
        assert_eq!(
            Proposal::Diff {
                path: s(""),
                lines: vec![]
            }
            .shown_bytes(),
            0
        );
    }

    #[test]
    fn every_string_of_a_proposal_is_made_visible() {
        let hid = "x\u{202e}y";
        let shown = r"x\u{202e}y";
        assert_eq!(
            Proposal::Command(s(hid)).escaped(),
            Proposal::Command(s(shown))
        );
        assert_eq!(Proposal::Plan(s(hid)).escaped(), Proposal::Plan(s(shown)));
        assert_eq!(Proposal::Other(s(hid)).escaped(), Proposal::Other(s(shown)));
        assert_eq!(
            Proposal::Diff {
                path: s(hid),
                lines: vec![
                    Hunk(s(hid)),
                    Context(s(hid)),
                    Removed(s(hid)),
                    Added(s(hid)),
                    Added(s("plain")),
                ]
            }
            .escaped(),
            Proposal::Diff {
                path: s(shown),
                lines: vec![
                    Hunk(s(shown)),
                    Context(s(shown)),
                    Removed(s(shown)),
                    Added(s(shown)),
                    Added(s("plain")),
                ]
            }
        );
    }

    #[test]
    fn a_command_is_shown_as_is() {
        for kind in [Some("execute"), None] {
            let p =
                Proposal::from_request(kind, &json!({ "command": "touch x", "description": "d" }));
            assert_eq!(p, Proposal::Command(s("touch x")));
        }
    }

    #[test]
    fn a_claude_edit_is_a_diff_of_the_two_strings() {
        let raw = json!({ "file_path": "/etc/nixos/a.nix", "old_string": "one\ntwo\nthree", "new_string": "one\nTWO\nthree" });
        assert_eq!(
            Proposal::from_request(Some("edit"), &raw),
            Proposal::Diff {
                path: s("/etc/nixos/a.nix"),
                lines: vec![
                    Context(s("one")),
                    Removed(s("two")),
                    Added(s("TWO")),
                    Context(s("three"))
                ]
            }
        );
    }

    #[test]
    fn a_new_file_is_all_added() {
        let raw = json!({ "file_path": "/tmp/plan.txt", "content": "hi\nthere\n" });
        assert_eq!(
            Proposal::from_request(Some("edit"), &raw),
            Proposal::Diff {
                path: s("/tmp/plan.txt"),
                lines: vec![Added(s("hi")), Added(s("there"))]
            }
        );
    }

    #[test]
    fn a_multi_edit_shows_each_edit() {
        let raw = json!({ "file_path": "f", "edits": [
            { "old_string": "a", "new_string": "b" }, { "old_string": "c", "new_string": "c\nd" } ] });
        assert_eq!(
            Proposal::from_request(Some("edit"), &raw),
            Proposal::Diff {
                path: s("f"),
                lines: vec![
                    Hunk(s("edit 1")),
                    Removed(s("a")),
                    Added(s("b")),
                    Hunk(s("edit 2")),
                    Context(s("c")),
                    Added(s("d"))
                ]
            }
        );
    }

    /// The request OpenCode 1.19.0 sent live (`p0c-opencode-edit`).
    #[test]
    fn an_opencode_edit_shows_its_unified_diff_without_headers() {
        let diff = "Index: /s/existing.txt\n===================================================================\n--- /s/existing.txt\n+++ /s/existing.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n\\ No newline at end of file\n";
        let raw = json!({ "filepath": "/s/existing.txt", "diff": diff });
        assert_eq!(
            Proposal::from_request(Some("edit"), &raw),
            Proposal::Diff {
                path: s("/s/existing.txt"),
                lines: vec![
                    Hunk(s("@@ -1,2 +1,2 @@")),
                    Context(s("one")),
                    Removed(s("two")),
                    Added(s("TWO"))
                ]
            }
        );
    }

    #[test]
    fn a_plan_exit_shows_the_plan_or_that_there_is_none() {
        let raw = json!({ "plan": "# Plan\n1. step", "planFilePath": "/p.md" });
        assert_eq!(
            Proposal::from_request(Some("switch_mode"), &raw),
            Proposal::Plan(s("# Plan\n1. step"))
        );
        assert_eq!(
            Proposal::from_request(Some("switch_mode"), &json!({})),
            Proposal::Plan(String::new())
        );
    }

    #[test]
    fn anything_else_is_shown_as_json() {
        let raw = json!({ "url": "https://example.org" });
        assert_eq!(
            Proposal::from_request(Some("fetch"), &raw),
            Proposal::Other(serde_json::to_string_pretty(&raw).unwrap())
        );
    }

    #[test]
    fn the_diff_keeps_common_lines() {
        let lines = line_diff("a\nb\nc\nd", "a\nc\nd\ne");
        assert_eq!(
            lines,
            vec![
                Context(s("a")),
                Removed(s("b")),
                Context(s("c")),
                Context(s("d")),
                Added(s("e"))
            ]
        );
    }

    #[test]
    fn a_huge_diff_falls_back_to_remove_then_add() {
        let old = "x\n".repeat(1001);
        let new = "x\n".repeat(1000) + "y\n";
        let lines = line_diff(&old, &new);
        assert_eq!(lines.len(), 1001 + 1001);
        assert!(matches!(lines[0], Removed(_)) && matches!(lines[1001], Added(_)));
    }
}
