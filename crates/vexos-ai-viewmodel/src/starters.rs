//! The starter tiles: a few ways into a task, shown in the top-bar panel and in
//! the chat's empty page.
//!
//! A tile is an id. The Shell extension sends only the id (the `draft` app
//! action); the text comes from here, fixed in the program, so nothing the
//! extension or an agent says can become a prompt. The text goes into the input
//! as a draft, like a notification's: the user reads it, may change it, and
//! sends it with the Send button, or does not.
//!
//! The ids and titles are the same list as the extension's `starters.js`;
//! `tests/extension/starters.test.mjs` fails if the two drift apart.

/// One tile.
#[derive(Debug, PartialEq, Eq)]
pub struct Starter {
    pub id: &'static str,
    pub title: &'static str,
    /// The draft. It asks the agent to ask first and to say what it will do
    /// before it does it, and never mentions applying a change.
    pub prompt: &'static str,
}

impl Starter {
    const fn new(id: &'static str, title: &'static str, prompt: &'static str) -> Starter {
        Starter { id, title, prompt }
    }
}

const DIAGNOSE: &str = "Something on this computer is not working right. Ask me what I am seeing, \
then check the system for failed services, recent crashes and errors, and tell me what you find \
before you change anything.";

const CHANGE: &str = "I want to change something about this system. Ask me what I would like, \
then make the change the way the vexos skill describes and dry-build it. Tell me what you will \
write before you write it, and leave the rebuild to me.";

const EXPLAIN: &str = "Explain what changed in the last rebuild of this system, and whether \
anything in it needs my attention.";

pub const STARTERS: [Starter; 3] = [
    Starter::new("diagnose", "Diagnose a problem", DIAGNOSE),
    Starter::new("change", "Change this system", CHANGE),
    Starter::new("explain", "Explain the last rebuild", EXPLAIN),
];

/// The starter with this id, if there is one. Anything else is nothing: an
/// unknown id does nothing at all.
pub fn starter(id: &str) -> Option<&'static Starter> {
    STARTERS.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_finds_its_starter_and_nothing_else_does() {
        for s in &STARTERS {
            assert_eq!(starter(s.id), Some(s));
        }
        for bad in [
            "",
            "Diagnose",
            "diagnose ",
            "rm -rf /",
            "diagnose\n",
            "../diagnose",
            "x",
        ] {
            assert_eq!(starter(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn ids_and_titles_are_unique_and_plain() {
        for (i, s) in STARTERS.iter().enumerate() {
            assert!(s.id.bytes().all(|b| b.is_ascii_lowercase()), "{}", s.id);
            assert!(
                s.title
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == ' ')
            );
            for other in &STARTERS[i + 1..] {
                assert_ne!(s.id, other.id);
                assert_ne!(s.title, other.title);
            }
        }
    }

    #[test]
    fn prompts_ask_first_and_never_apply_a_change() {
        for s in &STARTERS {
            let p = s.prompt.to_ascii_lowercase();
            assert!(!s.prompt.contains('\n') && s.prompt.len() < 600, "{}", s.id);
            for forbidden in [
                "nixos-rebuild",
                "just rebuild",
                "switch",
                "sudo",
                "pkexec",
                "--yolo",
            ] {
                assert!(!p.contains(forbidden), "{} names {forbidden}", s.id);
            }
        }
        assert!(
            starter("change")
                .unwrap()
                .prompt
                .contains("leave the rebuild to me")
        );
        assert!(
            starter("diagnose")
                .unwrap()
                .prompt
                .contains("before you change anything")
        );
    }
}
