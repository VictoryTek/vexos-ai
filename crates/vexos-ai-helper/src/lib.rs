//! The VexOS assistant helper: a session D-Bus service that only the GNOME
//! Shell may call. This crate is the front door (who may call, and what a call
//! may carry) and the session behind it: the agent's session, the conversation
//! and the composer, driven only by calls that passed the front door.

pub mod approval;
pub mod bus;
pub mod gate;
pub mod peers;
pub mod rate;
pub mod service;
