//! The rules of the helper's front door, with no D-Bus in them: who may call,
//! and what a call may carry. The bus layer reads a call into plain values,
//! asks this module, and only a call that passed reaches a handler.
//!
//! Every refusal is fail-closed: a missing sender, an unknown owner, a failed
//! lookup or an unexpected value all refuse.

/// The most text one `Edited` or `Send` may carry, in bytes. Over it, the call
/// is refused and the text is never cut.
pub const MAX_TEXT_BYTES: usize = 60_000;

/// Every method the helper has, and no other.
pub const METHODS: [&str; 10] = [
    "Open",
    "Close",
    "Edited",
    "Send",
    "Cancel",
    "CheckUsage",
    "DraftChoice",
    "DraftDiscard",
    "Decide",
    "Shown",
];

/// How the user sent a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Button,
    Enter,
}

/// What to do with a draft that arrived while the input had text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Replace,
    Append,
    Keep,
}

/// The only two answers to an approval request. There is no "always".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    ApproveOnce,
    DenyOnce,
}

/// A call that passed the argument rules, with typed arguments: nothing here
/// can carry a string the rules did not allow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Open,
    Close,
    Edited(String),
    Send {
        text: String,
        via: Via,
    },
    Cancel,
    CheckUsage,
    DraftChoice(Placement),
    DraftDiscard,
    Decide {
        generation: u64,
        request: u64,
        choice: Choice,
    },
    Shown {
        generation: u64,
        request: u64,
    },
    /// Not a method: an `Edited` the helper refused (too long, or too many), so
    /// the handler knows the input holds text it has not seen. Built by the bus
    /// layer after the caller check, never by [`Call::parse`].
    EditLost,
}

/// The arguments of a call, as the bus layer read them: strings and numbers in
/// the order they appear in the method's signature.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub strings: Vec<String>,
    pub numbers: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgError {
    UnknownMethod,
    TooLong,
    InvalidArgument,
}

impl Call {
    pub fn name(&self) -> &'static str {
        match self {
            Call::Open => "Open",
            Call::Close => "Close",
            Call::Edited(_) => "Edited",
            Call::Send { .. } => "Send",
            Call::Cancel => "Cancel",
            Call::CheckUsage => "CheckUsage",
            Call::DraftChoice(_) => "DraftChoice",
            Call::DraftDiscard => "DraftDiscard",
            Call::Decide { .. } => "Decide",
            Call::Shown { .. } => "Shown",
            Call::EditLost => "EditLost",
        }
    }

    /// The call `method` with `args`, or why it is refused. Each method has
    /// exactly one shape of arguments; anything else is refused, whatever the
    /// bus layer already checked.
    pub fn parse(method: &str, args: &Args) -> Result<Call, ArgError> {
        let (s, n) = (args.strings.as_slice(), args.numbers.as_slice());
        let shape = |strings: usize, numbers: usize| {
            if s.len() == strings && n.len() == numbers {
                Ok(())
            } else {
                Err(ArgError::InvalidArgument)
            }
        };
        match method {
            "Open" => shape(0, 0).map(|()| Call::Open),
            "Close" => shape(0, 0).map(|()| Call::Close),
            "Edited" => {
                shape(1, 0)?;
                Ok(Call::Edited(text(&s[0])?))
            }
            "Send" => {
                shape(2, 0)?;
                let via = match s[1].as_str() {
                    "button" => Via::Button,
                    "enter" => Via::Enter,
                    _ => return Err(ArgError::InvalidArgument),
                };
                Ok(Call::Send {
                    text: text(&s[0])?,
                    via,
                })
            }
            "Cancel" => shape(0, 0).map(|()| Call::Cancel),
            "CheckUsage" => shape(0, 0).map(|()| Call::CheckUsage),
            "DraftChoice" => {
                shape(1, 0)?;
                let placement = match s[0].as_str() {
                    "replace" => Placement::Replace,
                    "append" => Placement::Append,
                    "keep" => Placement::Keep,
                    _ => return Err(ArgError::InvalidArgument),
                };
                Ok(Call::DraftChoice(placement))
            }
            "DraftDiscard" => shape(0, 0).map(|()| Call::DraftDiscard),
            "Decide" => {
                shape(1, 2)?;
                let choice = match s[0].as_str() {
                    "approve-once" => Choice::ApproveOnce,
                    "deny-once" => Choice::DenyOnce,
                    _ => return Err(ArgError::InvalidArgument),
                };
                Ok(Call::Decide {
                    generation: n[0],
                    request: n[1],
                    choice,
                })
            }
            "Shown" => {
                shape(0, 2)?;
                Ok(Call::Shown {
                    generation: n[0],
                    request: n[1],
                })
            }
            _ => Err(ArgError::UnknownMethod),
        }
    }
}

/// `text` whole, or `TooLong`: text is never cut.
fn text(text: &str) -> Result<String, ArgError> {
    if text.len() > MAX_TEXT_BYTES {
        Err(ArgError::TooLong)
    } else {
        Ok(text.to_owned())
    }
}

/// What the bus and `/proc` said about a caller's process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub uid: u32,
    /// The file name (not the path) of the program the process runs.
    pub exe: String,
}

/// Why a caller's process could not be identified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupFailure {
    /// The credentials call failed or its reply was not what the bus documents.
    Credentials,
    /// The bus gave no user id.
    NoUserId,
    /// The bus gave no process file descriptor.
    NoProcessFd,
    /// The process is gone, or its descriptor could not be read.
    ProcessGone,
    /// The process id and the descriptor disagree.
    Inconsistent,
    /// The program path could not be read, was deleted, or had no usable name.
    Program,
}

/// Why a call was refused at the door. The reply to the caller is always the
/// same fixed error, whatever the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NoSender,
    NoShell,
    NotShell,
    Lookup(LookupFailure),
    WrongUser,
    WrongProgram,
}

pub struct Gate {
    own_uid: u32,
    programs: Vec<String>,
}

impl Gate {
    /// A gate for a helper run by user `own_uid`, which accepts the GNOME Shell
    /// only when it runs one of `programs` (file names, matched exactly).
    pub fn new(own_uid: u32, programs: &[&str]) -> Gate {
        Gate {
            own_uid,
            programs: programs.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    /// Whether the caller `sender` may call: it must be the current owner of
    /// `org.gnome.Shell`, run by the same user, and run one of the allowed
    /// programs. `identity` is what was learned about it, or why not.
    pub fn check_caller(
        &self,
        sender: Option<&str>,
        shell_owner: Option<&str>,
        identity: &Result<Identity, LookupFailure>,
    ) -> Result<(), Refusal> {
        let sender = sender.filter(|s| !s.is_empty()).ok_or(Refusal::NoSender)?;
        let owner = shell_owner
            .filter(|s| !s.is_empty())
            .ok_or(Refusal::NoShell)?;
        if sender != owner {
            return Err(Refusal::NotShell);
        }
        let who = identity.as_ref().map_err(|f| Refusal::Lookup(*f))?;
        if who.uid != self.own_uid {
            return Err(Refusal::WrongUser);
        }
        if !self.programs.contains(&who.exe) {
            return Err(Refusal::WrongProgram);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: &str = ":1.5";
    const PROGRAMS: [&str; 2] = ["gnome-shell", ".gnome-shell-wrapped"];

    fn gate() -> Gate {
        Gate::new(1000, &PROGRAMS)
    }

    fn who(uid: u32, exe: &str) -> Result<Identity, LookupFailure> {
        Ok(Identity {
            uid,
            exe: exe.to_owned(),
        })
    }

    // The caller.

    #[test]
    fn the_shell_run_by_the_same_user_passes() {
        for exe in PROGRAMS {
            assert_eq!(
                gate().check_caller(Some(SHELL), Some(SHELL), &who(1000, exe)),
                Ok(())
            );
        }
    }

    #[test]
    fn each_rule_refuses_on_its_own() {
        let g = gate();
        let ok = who(1000, "gnome-shell");
        assert_eq!(
            g.check_caller(None, Some(SHELL), &ok),
            Err(Refusal::NoSender)
        );
        assert_eq!(
            g.check_caller(Some(""), Some(""), &ok),
            Err(Refusal::NoSender)
        );
        assert_eq!(
            g.check_caller(Some(SHELL), None, &ok),
            Err(Refusal::NoShell)
        );
        assert_eq!(
            g.check_caller(Some(SHELL), Some(""), &ok),
            Err(Refusal::NoShell)
        );
        assert_eq!(
            g.check_caller(Some(":1.6"), Some(SHELL), &ok),
            Err(Refusal::NotShell)
        );
        assert_eq!(
            g.check_caller(Some(SHELL), Some(SHELL), &who(1001, "gnome-shell")),
            Err(Refusal::WrongUser)
        );
        assert_eq!(
            g.check_caller(Some(SHELL), Some(SHELL), &who(1000, "bash")),
            Err(Refusal::WrongProgram)
        );
    }

    #[test]
    fn every_failed_lookup_refuses() {
        for failure in [
            LookupFailure::Credentials,
            LookupFailure::NoUserId,
            LookupFailure::NoProcessFd,
            LookupFailure::ProcessGone,
            LookupFailure::Inconsistent,
            LookupFailure::Program,
        ] {
            assert_eq!(
                gate().check_caller(Some(SHELL), Some(SHELL), &Err(failure)),
                Err(Refusal::Lookup(failure))
            );
        }
    }

    #[test]
    fn a_program_must_match_exactly() {
        for exe in [
            "gnome-shell-evil",
            "xgnome-shell",
            "gnome-shell ",
            " gnome-shell",
            "GNOME-SHELL",
            "gnome-shell (deleted)",
            "gnome-shell\n",
            "",
            ".gnome-shell-wrapped-2",
            "gnome-shell/../bash",
        ] {
            assert_eq!(
                gate().check_caller(Some(SHELL), Some(SHELL), &who(1000, exe)),
                Err(Refusal::WrongProgram),
                "{exe:?}"
            );
        }
    }

    #[test]
    fn a_gate_with_no_programs_refuses_everything() {
        let g = Gate::new(1000, &[]);
        assert_eq!(
            g.check_caller(Some(SHELL), Some(SHELL), &who(1000, "gnome-shell")),
            Err(Refusal::WrongProgram)
        );
    }

    #[test]
    fn a_call_passes_only_when_every_rule_holds_together() {
        let g = gate();
        let senders = [None, Some(""), Some(SHELL), Some(":1.6")];
        let owners = [None, Some(""), Some(SHELL), Some(":1.6")];
        let identities = [
            Err(LookupFailure::Credentials),
            who(1000, "gnome-shell"),
            who(1000, "bash"),
            who(0, "gnome-shell"),
        ];
        let mut passed = 0;
        for sender in senders {
            for owner in owners {
                for identity in &identities {
                    let verdict = g.check_caller(sender, owner, identity);
                    // the owner is whoever owns the name: the same connection passes
                    let expect = sender.is_some_and(|s| !s.is_empty())
                        && sender == owner
                        && *identity == who(1000, "gnome-shell");
                    assert_eq!(verdict.is_ok(), expect, "{sender:?} {owner:?} {identity:?}");
                    passed += usize::from(verdict.is_ok());
                }
            }
        }
        // the owner :1.5 and the owner :1.6, each with the matching sender
        assert_eq!(passed, 2);
    }

    // The call.

    fn args(strings: &[&str], numbers: &[u64]) -> Args {
        Args {
            strings: strings.iter().map(|s| (*s).to_owned()).collect(),
            numbers: numbers.to_vec(),
        }
    }

    fn parse(method: &str, strings: &[&str], numbers: &[u64]) -> Result<Call, ArgError> {
        Call::parse(method, &args(strings, numbers))
    }

    #[test]
    fn every_method_parses_with_the_right_arguments() {
        assert_eq!(parse("Open", &[], &[]), Ok(Call::Open));
        assert_eq!(parse("Close", &[], &[]), Ok(Call::Close));
        assert_eq!(parse("Edited", &["hi"], &[]), Ok(Call::Edited("hi".into())));
        assert_eq!(
            parse("Send", &["hi", "button"], &[]),
            Ok(Call::Send {
                text: "hi".into(),
                via: Via::Button
            })
        );
        assert_eq!(
            parse("Send", &["hi", "enter"], &[]),
            Ok(Call::Send {
                text: "hi".into(),
                via: Via::Enter
            })
        );
        assert_eq!(parse("Cancel", &[], &[]), Ok(Call::Cancel));
        assert_eq!(parse("CheckUsage", &[], &[]), Ok(Call::CheckUsage));
        assert_eq!(
            parse("DraftChoice", &["replace"], &[]),
            Ok(Call::DraftChoice(Placement::Replace))
        );
        assert_eq!(
            parse("DraftChoice", &["append"], &[]),
            Ok(Call::DraftChoice(Placement::Append))
        );
        assert_eq!(
            parse("DraftChoice", &["keep"], &[]),
            Ok(Call::DraftChoice(Placement::Keep))
        );
        assert_eq!(parse("DraftDiscard", &[], &[]), Ok(Call::DraftDiscard));
        assert_eq!(
            parse("Decide", &["approve-once"], &[3, 7]),
            Ok(Call::Decide {
                generation: 3,
                request: 7,
                choice: Choice::ApproveOnce
            })
        );
        assert_eq!(
            parse("Decide", &["deny-once"], &[u64::MAX, 0]),
            Ok(Call::Decide {
                generation: u64::MAX,
                request: 0,
                choice: Choice::DenyOnce
            })
        );
        assert_eq!(
            parse("Shown", &[], &[1, 2]),
            Ok(Call::Shown {
                generation: 1,
                request: 2
            })
        );
    }

    #[test]
    fn the_methods_are_exactly_the_listed_ten() {
        assert_eq!(METHODS.len(), 10);
        let calls = [
            Call::Open,
            Call::Close,
            Call::Edited(String::new()),
            Call::Send {
                text: String::new(),
                via: Via::Button,
            },
            Call::Cancel,
            Call::CheckUsage,
            Call::DraftChoice(Placement::Keep),
            Call::DraftDiscard,
            Call::Decide {
                generation: 0,
                request: 0,
                choice: Choice::DenyOnce,
            },
            Call::Shown {
                generation: 0,
                request: 0,
            },
        ];
        let names: Vec<&str> = calls.iter().map(Call::name).collect();
        assert_eq!(names, METHODS);
    }

    #[test]
    fn an_unknown_method_is_refused() {
        for method in [
            "Frobnicate",
            "open",
            "OPEN",
            "Open ",
            "",
            "Open\n",
            "Send2",
            "Eval",
        ] {
            assert_eq!(
                parse(method, &[], &[]),
                Err(ArgError::UnknownMethod),
                "{method:?}"
            );
        }
    }

    #[test]
    fn the_wrong_number_of_arguments_is_refused() {
        for (method, strings, numbers) in [
            ("Open", vec!["x"], vec![]),
            ("Open", vec![], vec![1]),
            ("Close", vec!["x"], vec![]),
            ("Edited", vec![], vec![]),
            ("Edited", vec!["a", "b"], vec![]),
            ("Edited", vec!["a"], vec![1]),
            ("Send", vec!["a"], vec![]),
            ("Send", vec!["a", "button", "x"], vec![]),
            ("Cancel", vec!["x"], vec![]),
            ("CheckUsage", vec!["/usage"], vec![]),
            ("CheckUsage", vec!["x"], vec![]),
            ("DraftChoice", vec![], vec![]),
            ("DraftChoice", vec!["keep", "x"], vec![]),
            ("DraftDiscard", vec!["x"], vec![]),
            ("Decide", vec!["deny-once"], vec![1]),
            ("Decide", vec!["deny-once"], vec![1, 2, 3]),
            ("Decide", vec![], vec![1, 2]),
            ("Decide", vec!["deny-once", "x"], vec![1, 2]),
            ("Shown", vec![], vec![1]),
            ("Shown", vec!["x"], vec![1, 2]),
        ] {
            assert_eq!(
                parse(method, &strings, &numbers),
                Err(ArgError::InvalidArgument),
                "{method} {strings:?} {numbers:?}"
            );
        }
    }

    #[test]
    fn text_over_the_limit_is_refused_and_never_cut() {
        for method in ["Edited", "Send"] {
            let strings = |text: String| -> Vec<String> {
                if method == "Send" {
                    vec![text, "button".to_owned()]
                } else {
                    vec![text]
                }
            };
            let run = |text: String| {
                Call::parse(
                    method,
                    &Args {
                        strings: strings(text),
                        numbers: vec![],
                    },
                )
            };
            assert!(run("a".repeat(MAX_TEXT_BYTES)).is_ok(), "{method}");
            assert_eq!(
                run("a".repeat(MAX_TEXT_BYTES + 1)),
                Err(ArgError::TooLong),
                "{method}"
            );
            // bytes, not characters: 20,000 three-byte characters are exactly 60,000
            assert!(run("日".repeat(20_000)).is_ok(), "{method}");
            assert_eq!(run("日".repeat(20_001)), Err(ArgError::TooLong), "{method}");
            assert_eq!(run("😀".repeat(15_001)), Err(ArgError::TooLong), "{method}");
            assert!(run(String::new()).is_ok(), "{method}");
        }
        // and the text that passes is the text that was sent, whole
        let text = "a".repeat(MAX_TEXT_BYTES);
        assert_eq!(
            parse("Edited", &[&text], &[]),
            Ok(Call::Edited(text.clone()))
        );
    }

    #[test]
    fn only_the_two_vias_are_accepted() {
        for via in [
            "Button",
            "ENTER",
            "",
            "click",
            "button ",
            "button\n",
            "enter-key",
        ] {
            assert_eq!(
                parse("Send", &["hi", via], &[]),
                Err(ArgError::InvalidArgument),
                "{via:?}"
            );
        }
    }

    #[test]
    fn only_the_three_placements_are_accepted() {
        for choice in ["Replace", "cancel", "", "append ", "KEEP", "discard"] {
            assert_eq!(
                parse("DraftChoice", &[choice], &[]),
                Err(ArgError::InvalidArgument),
                "{choice:?}"
            );
        }
    }

    #[test]
    fn only_approve_once_and_deny_once_decide_a_request() {
        for choice in [
            "approve-always",
            "approve",
            "allow",
            "Approve-once",
            "approve-once\n",
            "deny",
            "",
            "approve_once",
            "always",
        ] {
            assert_eq!(
                parse("Decide", &[choice], &[1, 1]),
                Err(ArgError::InvalidArgument),
                "{choice:?}"
            );
        }
    }

    #[test]
    fn a_too_long_text_and_a_bad_via_are_both_refused() {
        let long = "a".repeat(MAX_TEXT_BYTES + 1);
        assert!(parse("Send", &[&long, "button"], &[]).is_err());
        assert!(parse("Send", &["hi", "x"], &[]).is_err());
        assert!(parse("Send", &[&long, "x"], &[]).is_err());
    }
}
