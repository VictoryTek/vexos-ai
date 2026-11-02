//! The helper's program. It owns its name, refuses everyone but the GNOME
//! Shell, and runs the session for the Shell's calls. It exits when no `Open`
//! comes, when the session is closed, when the Shell goes away, or when it
//! loses its name.

use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vexos_ai_helper::bus::{BUS_NAME, Bus, Config, Exit, Handler, Helper};
use vexos_ai_helper::peers::own_uid;
use vexos_ai_helper::service::{Agents, Service, Settings};
use vexos_ai_viewmodel::{AGENT_FILE, PANEL_SETTINGS_FILE, PanelSettings, saved_agent};

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
        clock: Rc::new(Instant::now),
    };

    // The core is async on tokio; GLib owns the main thread.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(why) => {
            eprintln!("vexos-ai-helper: cannot start the runtime: {why}");
            return ExitCode::FAILURE;
        }
    };
    // The user's own files, read once and never written: the saved agent, and
    // whether `/usage` runs when the session is ready.
    let settings = Settings {
        agent: saved_agent(&glib::user_config_dir().join(AGENT_FILE)),
        usage_on_launch: std::fs::read_to_string(glib::user_config_dir().join(PANEL_SETTINGS_FILE))
            .map(|text| PanelSettings::parse(&text))
            .unwrap_or_default()
            .usage_on_launch,
    };
    // This process runs one session. Its number is the start time in
    // nanoseconds, so a later helper never reuses it and a late call aimed at
    // an earlier session cannot answer a request of this one.
    let generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |t| u64::try_from(t.as_nanos()).unwrap_or(u64::MAX))
        .max(1);
    let service = Rc::new(Service::new(
        runtime.handle().clone(),
        Box::new(Agents::new(runtime.handle().clone())),
        settings,
        generation,
        Rc::new(Instant::now),
    ));
    let handler: Rc<dyn Handler> = service.clone();

    let started = glib::MainContext::default().block_on(Helper::start(config, handler));
    let helper = match started {
        Ok(helper) => helper,
        Err(why) => {
            eprintln!("vexos-ai-helper: cannot start: {why:?}");
            return ExitCode::FAILURE;
        }
    };
    let exit = helper.run();
    // Whatever the reason, nothing stays pending: the session is closed, which
    // denies what is waiting.
    service.end();
    match exit {
        Exit::NameLost => ExitCode::FAILURE,
        Exit::NoOpen | Exit::Quit | Exit::ShellGone => ExitCode::SUCCESS,
    }
}
