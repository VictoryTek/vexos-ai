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
//! - The input ([`Composer`]) is the only source of typed text to send: a
//!   draft needs the Send button until the user edits it, and never
//!   replaces typed text without asking. The one other message is the
//!   constant [`USAGE_COMMAND`] from [`Conversation::check_usage`]: Claude
//!   only, when ready with no turn running and no approval waiting. It is
//!   sent once when the session becomes ready (hidden: its reply goes only
//!   to the usage parser), and on a click on the usage line (shown, reply in
//!   the transcript). Nothing else sends it.
//! - The agent is the saved choice (`vexos-ai pick`, VexPortal), read-only: the
//!   two known values, else Claude ([`saved_agent`]). There is no switcher;
//!   starting a session sends nothing. The launch timing is display text only.
//! - The window mode (docked under the top bar, or floating) is one of two
//!   fixed strings in a GSettings key shared with the Shell extension; any
//!   other value means docked. The toggle appears only with the extension
//!   running.
//! - The usage line shows the 5-hour limit read by a strict parser from a
//!   `/usage` reply, or "unknown". It is display text only: never sent,
//!   logged or stored, and nothing else depends on it.

mod composer;
mod conversation;
mod diagnose;
mod markdown;
mod notify;
mod proposal;
mod rebuild;
mod redact;
mod usage;
mod window;

pub use composer::{Composer, Draft, Offer, Outgoing, Placement, Via};
pub use conversation::{
    AGENT_FILE, Answer, ApprovalCard, ClickError, Conversation, Decider, Item, NO_REPLY, NOT_SENT,
    Sent, State, ToolCard, USAGE_COMMAND, USAGE_NOT_SENT, agent_label, choice_label, mode_label,
    parse_agent, saved_agent,
};
pub use diagnose::{
    BEGIN, Crash, END, Excerpt, INSTRUCTION, Log, Missing, Subject, UnitName, core_file,
    coredump_argv, coredump_log, draft, excerpt, journal_argv, journal_log,
};
pub use markdown::{Run, Style, markdown};
pub use notify::{Crashes, FailedUnits, SETTINGS_FILE, Settings};
pub use proposal::{DiffLine, Proposal, line_diff};
pub use rebuild::{Rebuild, RebuildCommand, describe_wait_status};
pub use usage::{
    CLICK_TOOLTIP, Disabled, FiveHour, PANEL_SETTINGS_FILE, PanelSettings, USAGE_LOCK, UsageButton,
    five_hour, freshness, tooltip,
};
pub use window::{
    EXTENSION_BUS_NAME, WINDOW_MODE_KEY, WINDOW_SCHEMA, WindowLook, WindowMode, window_look,
};
