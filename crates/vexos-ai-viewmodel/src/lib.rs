//! What the assistant panel shows, kept apart from GTK so it can be tested
//! headless. The window renders these types and forwards clicks; it decides
//! nothing itself.
//!
//! - A conversation is fed only by the core's [`vexos_ai_core::Event`]s.
//! - A permission request is answered only by [`Conversation::click`]. Closing
//!   denies whatever is still pending.
//! - Approval cards offer exactly the choices the core passed up: "Approve
//!   once" and "Deny once". There is nothing to set a mode with; the mode is
//!   what the core last confirmed.
//! - Agent replies are formatted as literal text runs; nothing in one can
//!   become markup, a clickable link or a loaded image.
//! - Rebuilding the system is a separate state machine with a fixed command;
//!   nothing in a conversation can start it.
//! - A failure notification leads to a draft for the input box, never to a
//!   prompt: collected text is redacted, capped and fenced as untrusted, and
//!   the user decides whether to send it.
//! - The input ([`Composer`]) is the only source of a message to send: a
//!   draft needs the Send button until the user edits it, and never
//!   replaces typed text without asking.

mod composer;
mod conversation;
mod diagnose;
mod markdown;
mod notify;
mod proposal;
mod rebuild;
mod redact;

pub use composer::{Composer, Draft, Offer, Outgoing, Placement, Via};
pub use conversation::{
    Answer, ApprovalCard, ClickError, Conversation, Decider, Item, NOT_SENT, State, ToolCard,
    agent_label, choice_label, mode_label,
};
pub use diagnose::{
    BEGIN, Crash, END, Excerpt, INSTRUCTION, Log, Missing, Subject, UnitName, core_file,
    coredump_argv, coredump_log, draft, excerpt, journal_argv, journal_log,
};
pub use markdown::{Run, Style, markdown};
pub use notify::{Crashes, FailedUnits, SETTINGS_FILE, Settings};
pub use proposal::{DiffLine, Proposal, line_diff};
pub use rebuild::{Rebuild, RebuildCommand, describe_wait_status};
