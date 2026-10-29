//! Diagnosis notifications, when switched on: newly failed systemd units and
//! the user's own crashes.
//!
//! A notification carries a fixed title and a validated name, never log
//! text. Clicking it opens the window and collects read-only (fixed argv,
//! no shell, by the app and never the agent), then holds a draft for the
//! input. Nothing here sends anything to the agent: the user reviews the
//! draft and presses Send, or doesn't. Opening the window starts the saved
//! agent's session, as any opening does; starting sends nothing.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use vexos_ai_viewmodel::{
    Crash, Crashes, FailedUnits, Log, Missing, SETTINGS_FILE, Settings, Subject, UnitName,
    coredump_argv, coredump_log, draft, journal_argv, journal_log,
};

use crate::{Slot, Ui, window};

/// Where systemd-coredump links new core files.
const COREDUMP_DIR: &str = "/var/lib/systemd/coredump";
const POLL_SECONDS: u32 = 60;
const COLLECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Read once per process from `$XDG_CONFIG_HOME/vexos-ai/notifications.conf`.
pub(crate) fn settings() -> Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    *SETTINGS.get_or_init(|| {
        std::fs::read_to_string(glib::user_config_dir().join(SETTINGS_FILE))
            .map(|text| Settings::parse(&text))
            .unwrap_or_default()
    })
}

fn now() -> u64 {
    u64::try_from(glib::monotonic_time() / 1_000_000).unwrap_or(0)
}

/// This process's uid.
fn uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// What a notification click activates. Any client on the session bus can
/// activate these too, so each target is validated again, and all they can
/// do is put a draft in the input.
pub(crate) fn actions(slot: &Slot) -> Vec<gio::ActionEntry<adw::Application>> {
    let unit_action = gio::ActionEntry::builder("diagnose-unit")
        .parameter_type(Some(glib::VariantTy::STRING))
        .activate({
            let slot = slot.clone();
            move |app: &adw::Application, _, target| {
                let Some(unit) = target.and_then(|t| t.str()).and_then(UnitName::parse) else {
                    return;
                };
                let ui = window(app, &slot);
                ui.window.present();
                let argv = journal_argv(&unit);
                collect(ui, Subject::Unit(unit), argv, journal_log);
            }
        })
        .build();
    let coredump_action = gio::ActionEntry::builder("diagnose-coredump")
        .parameter_type(Some(glib::VariantTy::new("(us)").expect("a valid type")))
        .activate({
            let slot = slot.clone();
            move |app: &adw::Application, _, target| {
                let Some((pid, comm)) = target.and_then(|t| t.get::<(u32, String)>()) else {
                    return;
                };
                let (Some(crash), Some(uid)) = (Crash::from_parts(&comm, pid), uid()) else {
                    return;
                };
                let ui = window(app, &slot);
                ui.window.present();
                let argv = coredump_argv(crash.pid, uid);
                collect(ui, Subject::Coredump(crash), argv, coredump_log);
            }
        })
        .build();
    vec![unit_action, coredump_action]
}

/// Runs the fixed argv, then holds the draft in the window.
fn collect(ui: Rc<Ui>, subject: Subject, argv: Vec<String>, read: fn(bool, &[u8], &[u8]) -> Log) {
    glib::spawn_future_local(async move {
        let log = match run(&argv).await {
            Some((success, stdout, stderr)) => read(success, &stdout, &stderr),
            None => Log::Missing(Missing::Failed),
        };
        ui.hold_draft(draft(&subject, &log));
    });
}

/// Spawns argv directly (no shell, stdin from /dev/null), with a timeout.
async fn run(argv: &[String]) -> Option<(bool, Vec<u8>, Vec<u8>)> {
    let args: Vec<&OsStr> = argv.iter().map(OsStr::new).collect();
    let child = gio::Subprocess::newv(
        &args,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .ok()?;
    let bytes = |b: Option<glib::Bytes>| b.map(|b| b.to_vec()).unwrap_or_default();
    match glib::future_with_timeout(COLLECT_TIMEOUT, child.communicate_future(None)).await {
        Ok(Ok((stdout, stderr))) => Some((child.is_successful(), bytes(stdout), bytes(stderr))),
        Ok(Err(_)) => None,
        Err(_) => {
            child.force_exit();
            None
        }
    }
}

fn unit_failed(app: &adw::Application, unit: &UnitName) {
    let n = gio::Notification::new("A system service failed");
    n.set_body(Some(unit.as_str()));
    n.set_default_action_and_target_value("app.diagnose-unit", Some(&unit.as_str().to_variant()));
    app.send_notification(Some(&format!("unit-{}", unit.as_str())), &n);
}

fn crashed(app: &adw::Application, crash: &Crash) {
    let n = gio::Notification::new("A program crashed");
    let comm = crash.comm.as_deref().unwrap_or("");
    n.set_body(Some(&match comm {
        "" => format!("PID {}", crash.pid),
        comm => format!("{comm} (PID {})", crash.pid),
    }));
    n.set_default_action_and_target_value(
        "app.diagnose-coredump",
        Some(&(crash.pid, comm.to_owned()).to_variant()),
    );
    app.send_notification(Some(&format!("crash-{}", crash.pid)), &n);
}

/// Starts the watchers that are switched on. While one runs, the app stays
/// up after its window closes, so it keeps watching. It is not started at
/// login: watching begins when the panel is first opened.
pub(crate) fn start_watchers(app: &adw::Application) {
    let settings = settings();
    if settings.failed_units {
        watch_failed_units(app);
    }
    if settings.coredumps {
        watch_coredumps(app);
    }
}

/// Every minute, systemd's failed list over the system bus: a read-only
/// method call, no subprocess.
fn watch_failed_units(app: &adw::Application) {
    let app = app.clone();
    let hold = app.hold();
    glib::spawn_future_local(async move {
        let _hold = hold;
        let Ok(bus) = gio::bus_get_future(gio::BusType::System).await else {
            return;
        };
        let mut watch = FailedUnits::default();
        loop {
            if let Some(failed) = failed_units(&bus).await {
                for unit in watch.observe(&failed, now()) {
                    unit_failed(&app, &unit);
                }
            }
            glib::timeout_future_seconds(POLL_SECONDS).await;
        }
    });
}

async fn failed_units(bus: &gio::DBusConnection) -> Option<Vec<String>> {
    let reply = bus
        .call_future(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
            "ListUnitsFiltered",
            Some(&(vec!["failed"],).to_variant()),
            Some(glib::VariantTy::new("(a(ssssssouso))").expect("a valid type")),
            gio::DBusCallFlags::NO_AUTO_START,
            10_000,
        )
        .await
        .ok()?;
    Some(
        reply
            .child_value(0)
            .iter()
            .filter_map(|unit| unit.child_value(0).str().map(str::to_owned))
            .collect(),
    )
}

/// New files in systemd-coredump's directory, own uid only.
fn watch_coredumps(app: &adw::Application) {
    let Some(uid) = uid() else {
        return;
    };
    let Ok(monitor) = gio::File::for_path(COREDUMP_DIR).monitor_directory(
        gio::FileMonitorFlags::WATCH_MOVES,
        None::<&gio::Cancellable>,
    ) else {
        return;
    };
    let hold = app.hold();
    let app = app.clone();
    let crashes = RefCell::new(Crashes::new(uid));
    monitor.connect_changed(move |_, file, other, event| {
        let _ = &hold;
        let file = match event {
            gio::FileMonitorEvent::Created | gio::FileMonitorEvent::MovedIn => Some(file),
            gio::FileMonitorEvent::Renamed => other,
            _ => None,
        };
        let Some(name) = file.and_then(|f| f.basename()) else {
            return;
        };
        let Some(name) = name.to_str() else {
            return;
        };
        if let Some(crash) = crashes.borrow_mut().observe(name, now()) {
            crashed(&app, &crash);
        }
    });
    // Dropping the monitor stops it; it watches for the life of the process.
    std::mem::forget(monitor);
}
