//! "Open in a terminal": the same assistant, in a terminal window, by running
//! `vexos-ai` (the CLI the module installs, which finds a terminal itself and
//! starts the agent in /etc/nixos, asking before it acts).
//!
//! The program name is a constant, there are no arguments and no shell, and
//! nothing from the window, an agent or the user reaches it. It is not a way
//! to run anything else.

use std::ffi::OsStr;

use gtk::gio;

/// The one program this module runs.
const PROGRAM: &str = "vexos-ai";

pub(crate) fn open() {
    // A missing program or a failed start is not worth a dialog: the window
    // is still there. The child is left to run on its own.
    let _ = gio::Subprocess::newv(&[OsStr::new(PROGRAM)], gio::SubprocessFlags::NONE);
}
