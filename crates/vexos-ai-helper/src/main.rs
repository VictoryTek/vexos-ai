//! The helper's program. Until a session is wired in (a later item), it owns its
//! name, refuses everyone but the GNOME Shell, and answers every accepted call
//! with `NotReady`. It exits when no `Open` comes, or when it loses its name.

use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use vexos_ai_helper::bus::{BUS_NAME, Bus, Config, Exit, Helper, NotWired};
use vexos_ai_helper::peers::own_uid;

fn main() -> ExitCode {
    let status = std::fs::read_to_string("/proc/self/status").ok();
    let Some(uid) = status.as_deref().and_then(own_uid) else {
        eprintln!("vexos-ai-helper: cannot read its own user id");
        return ExitCode::FAILURE;
    };
    let config = Config {
        bus: Bus::Session,
        name: BUS_NAME.to_owned(),
        // NixOS wraps the Shell: its program file is `.gnome-shell-wrapped`.
        programs: vec!["gnome-shell".to_owned(), ".gnome-shell-wrapped".to_owned()],
        own_uid: uid,
        open_timeout: Duration::from_secs(15),
    };
    let started = glib::MainContext::default().block_on(Helper::start(config, Rc::new(NotWired)));
    let helper = match started {
        Ok(helper) => helper,
        Err(why) => {
            eprintln!("vexos-ai-helper: cannot start: {why:?}");
            return ExitCode::FAILURE;
        }
    };
    match helper.run() {
        Exit::NameLost => ExitCode::FAILURE,
        Exit::NoOpen | Exit::Quit => ExitCode::SUCCESS,
    }
}
