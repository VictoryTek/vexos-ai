//! The usage service program (Linux). A GApplication with no window, started
//! by D-Bus activation, that exits when it has been idle for 30 s.
//!
//! The Shell extension calls `ActivateAction("refresh")` (or `"refresh-now"`,
//! which skips the 15 s interval) on `io.github.victorytek.VexosAssistant.Usage`.
//! Whether usage is on is the extension's GSettings key `usage-enabled`, read
//! before anything else; a missing schema or key means off, so a broken
//! install reads nothing.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("vexos-ai-usage runs on Linux only");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
mod service {
    use std::cell::Cell;
    use std::rc::Rc;

    use gio::prelude::*;

    pub const APP_ID: &str = "io.github.victorytek.VexosAssistant.Usage";
    const SCHEMA: &str = "org.gnome.shell.extensions.vexos-assistant";
    const KEY: &str = "usage-enabled";
    const IDLE_MS: u32 = 30_000;

    /// The switch. Anything unexpected is off.
    fn enabled() -> bool {
        let Some(source) = gio::SettingsSchemaSource::default() else {
            return false;
        };
        let Some(schema) = source.lookup(SCHEMA, true) else {
            return false;
        };
        if !schema.has_key(KEY) {
            return false;
        }
        gio::Settings::new(SCHEMA).boolean(KEY)
    }

    /// One refresh at a time. A call while one runs is not run alongside it
    /// (two at once would ask an endpoint that rate-limits readily twice, from
    /// the same old record) and is not lost either: one more run is queued, and
    /// it reads the switch again. So "Show plan limits" then "Turn off" in quick
    /// succession still ends with the record cleared.
    struct Flight {
        busy: Cell<bool>,
        /// A run is queued: its `force`.
        again: Cell<Option<bool>>,
    }

    fn start(app: &gio::Application, flight: &Rc<Flight>, mut force: bool) {
        flight.busy.set(true);
        // Held while the requests run, so the idle timer cannot end them.
        let guard = app.hold();
        let flight = flight.clone();
        glib::spawn_future_local(async move {
            loop {
                let on = enabled();
                let done = gio::spawn_blocking(move || vexos_ai_usage::run(on, force)).await;
                // One line for the journal, so a panel that shows nothing can
                // be diagnosed: the switch and a fixed status word, no data.
                let outcome = match done {
                    Ok(Some(record)) => record.status.as_str(),
                    Ok(None) => "no-home",
                    Err(_) => "crashed",
                };
                eprintln!("vexos-ai-usage: enabled={on} force={force} status={outcome}");
                match flight.again.take() {
                    Some(queued) => force = queued,
                    None => break,
                }
            }
            flight.busy.set(false);
            drop(guard);
        });
    }

    fn action(app: &gio::Application, name: &str, force: bool, flight: &Rc<Flight>) {
        let simple = gio::SimpleAction::new(name, None);
        let weak = app.downgrade();
        let flight = flight.clone();
        simple.connect_activate(move |_, _| {
            let Some(app) = weak.upgrade() else { return };
            if flight.busy.get() {
                let queued = flight.again.get().unwrap_or(false) || force;
                flight.again.set(Some(queued));
            } else {
                start(&app, &flight, force);
            }
        });
        app.add_action(&simple);
    }

    pub fn run() -> glib::ExitCode {
        let app = gio::Application::builder()
            .application_id(APP_ID)
            .flags(gio::ApplicationFlags::IS_SERVICE)
            .inactivity_timeout(IDLE_MS)
            .build();
        app.connect_startup(|app| {
            let flight = Rc::new(Flight {
                busy: Cell::new(false),
                again: Cell::new(None),
            });
            action(app, "refresh", false, &flight);
            action(app, "refresh-now", true, &flight);
        });
        app.connect_activate(|_| {});
        app.run()
    }
}

#[cfg(target_os = "linux")]
fn main() -> glib::ExitCode {
    service::run()
}
