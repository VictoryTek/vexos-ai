//! The usage service: Claude's plan limits for the top-bar panel.
//!
//! It exists so that exactly one small, tested piece of this project ever
//! opens Claude's sign-in, and only when the user has switched plan limits on.
//! It asks Anthropic's usage endpoint once per request, and writes numbers and
//! fixed words to a private file that the Shell extension reads. See
//! `docs/contract.md`, "Usage service".
//!
//! The library builds and tests on any platform; the D-Bus service around it
//! (`src/main.rs`) is Linux only.

pub mod credentials;
pub mod parse;
pub mod probe;
pub mod refresh;
pub mod secret;
pub mod state;
pub mod time;

use std::path::PathBuf;

pub use refresh::{Env, refresh};
pub use state::{Record, Status};

/// The home directory: `$HOME`.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// One refresh against the real login, the real endpoint and the real state
/// file. `enabled` is the panel's switch; `force` skips the 15 s interval.
/// `None` when there is no home directory to work in.
pub fn run(enabled: bool, force: bool) -> Option<Record> {
    let home = home()?;
    let xdg = std::env::var("XDG_STATE_HOME").ok();
    let env = Env {
        now_ms: time::now_ms(),
        state_file: state::state_home(&home, xdg.as_deref()).join(state::FILE),
        claude_config_dir: std::env::var("CLAUDE_CONFIG_DIR").ok(),
        home,
        enabled,
        force,
    };
    Some(refresh(&env, &probe::Anthropic::new()))
}
