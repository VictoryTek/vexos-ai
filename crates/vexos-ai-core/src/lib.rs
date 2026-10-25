//! The VexOS assistant core: runs Claude Code or OpenCode over the Agent Client
//! Protocol (ACP) and keeps the safety rules of `nix/policy.nix` true for every
//! session it owns.
//!
//! - The mode is pinned (Claude `default`, OpenCode `build`) right after a
//!   session starts and whenever the agent reports another one. A failed reset
//!   aborts the session. There is no API to choose another mode or agent.
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
