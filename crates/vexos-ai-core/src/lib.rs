//! The VexOS assistant core: runs Claude Code or OpenCode over the Agent Client
//! Protocol (ACP) and keeps the safety rules of `nix/policy.nix` true for every
//! session it owns.
//!
//! - Every session starts in Claude `default` or OpenCode `build`. A mode
//!   report outside the allow-list (Claude `default`/`plan`, OpenCode `build`),
//!   or one that can't be read, resets it; a failed reset aborts the session.
//!   There is no API to choose a mode or agent; [`Event::Mode`] reports the
//!   mode the core has confirmed, read-only.
//! - Only "approve once" and "deny once" ever reach the UI.
//! - A permission request is approved only by an explicit [`Session::decide`].
//!   Timeouts, cancels, a dropped session and a closed event stream deny.
//! - The core sets no auto-approving mode, flag or environment variable.
//!
//! See `.github/docs/subagent_docs/assistant_panel_spec.md` and the spike
//! results next to it for why each rule exists.

mod agent;
mod permission;
mod rpc;
mod session;

pub use agent::{Agent, AgentCommand};
pub use permission::{Decision, PermissionRequest};
pub use session::{Config, Error, Event, Events, Session, ToolCall};
