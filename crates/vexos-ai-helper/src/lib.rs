//! The VexOS assistant helper: a session D-Bus service that only the GNOME
//! Shell may call. This crate is the front door (who may call, and what a call
//! may carry); it holds no session yet.

pub mod bus;
pub mod gate;
pub mod peers;
