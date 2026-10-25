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
//! - Rebuilding the system is a separate state machine with a fixed command;
//!   nothing in a conversation can start it.

mod conversation;
mod proposal;
mod rebuild;

pub use conversation::{
    Answer, ApprovalCard, ClickError, Conversation, Decider, Item, NOT_SENT, State, ToolCard,
    agent_label, choice_label, mode_label,
};
pub use proposal::{DiffLine, Proposal, line_diff};
pub use rebuild::{Rebuild, RebuildCommand, describe_wait_status};
