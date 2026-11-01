//! The helper on a private bus: a real `dbus-daemon`, started by each scenario
//! with a small config on a socket in its own temporary directory, and callers
//! that really are other connections and other processes.
//!
//! GLib objects belong to the thread that made them, and the helper lives on
//! the process's one default main context. So every scenario runs, in turn, on
//! this one test's thread; each is named in the output and a failure in one
//! does not hide the others.

use std::cell::RefCell;
use std::future::Future;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gio::prelude::*;
use gio::{DBusCallFlags, DBusConnection, DBusConnectionFlags};
use glib::{MainContext, Variant};
use vexos_ai_helper::bus::{
    BUS_NAME, Bus, Config, Exit, Handler, HandlerError, Helper, INTERFACE, INTERFACE_XML,
    OBJECT_PATH, Reply, SHELL_NAME, StartError,
};
use vexos_ai_helper::gate::{Call, Choice, METHODS, Placement, Via};
use vexos_ai_helper::peers::own_uid;

#[path = "../testing/calls.rs"]
mod calls;

const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
const NOT_READY: &str = "io.github.victorytek.VexosAssistant.Error.NotReady";
const TOO_LONG: &str = "io.github.victorytek.VexosAssistant.Error.TooLong";
const INVALID: &str = "io.github.victorytek.VexosAssistant.Error.InvalidArgument";

// A private bus.

struct TestBus {
    child: Child,
    _stdout: BufReader<ChildStdout>,
    dir: PathBuf,
    address: String,
}

static BUSES: AtomicUsize = AtomicUsize::new(0);

impl TestBus {
    fn start() -> TestBus {
        let dir = std::env::temp_dir().join(format!(
            "vexos-helper-bus-{}-{}",
            std::process::id(),
            BUSES.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("bus.conf");
        std::fs::write(
            &conf,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                dir.display()
            ),
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", conf.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon must be on PATH for these tests");
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert!(line.starts_with("unix:"), "bus address: {line:?}");
        TestBus {
            child,
            _stdout: stdout,
            dir,
            address: line.trim().to_owned(),
        }
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// Running things on the default main context, where the helper lives.

fn run<F: Future>(future: F) -> F::Output {
    MainContext::default().block_on(future)
}

async fn connect(address: &str) -> DBusConnection {
    DBusConnection::for_address_future(
        address,
        DBusConnectionFlags::AUTHENTICATION_CLIENT | DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
    )
    .await
    .expect("connect to the private bus")
}

/// Becomes the owner of `name`, or panics.
async fn own_name(conn: &DBusConnection, name: &str) {
    let reply = conn
        .call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(name, 4u32).to_variant()),
            None,
            DBusCallFlags::NONE,
            5000,
        )
        .await
        .expect("RequestName");
    assert_eq!(
        reply.get::<(u32,)>(),
        Some((1,)),
        "not the primary owner of {name}"
    );
}

async fn release_name(conn: &DBusConnection, name: &str) {
    conn.call_future(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ReleaseName",
        Some(&(name,).to_variant()),
        None,
        DBusCallFlags::NONE,
        5000,
    )
    .await
    .expect("ReleaseName");
}

fn error_name(e: &glib::Error) -> String {
    gio::DBusError::remote_error(e)
        .map(|name| name.to_string())
        .unwrap_or_else(|| format!("local: {e}"))
}

/// The reply of the helper's `method`: `Ok(())`, or the name of the error.
async fn call(conn: &DBusConnection, method: &str, params: Option<Variant>) -> Result<(), String> {
    call_at(conn, OBJECT_PATH, INTERFACE, method, params)
        .await
        .map(|_| ())
        .map_err(|e| error_name(&e))
}

async fn call_at(
    conn: &DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    params: Option<Variant>,
) -> Result<Variant, glib::Error> {
    conn.call_future(
        Some(BUS_NAME),
        path,
        interface,
        method,
        params.as_ref(),
        None,
        DBusCallFlags::NONE,
        5000,
    )
    .await
}

// The helper under test.

#[derive(Default)]
struct Recorder {
    calls: RefCell<Vec<Call>>,
}

impl Handler for Recorder {
    fn handle(&self, call: Call) -> Result<Reply, HandlerError> {
        self.calls.borrow_mut().push(call);
        Err(HandlerError::NotReady)
    }
}

fn this_program() -> String {
    file_name(&std::env::current_exe().unwrap())
}

fn child_program() -> String {
    file_name(Path::new(env!("CARGO_BIN_EXE_vexos-ai-helper-fakecaller")))
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_str().unwrap().to_owned()
}

fn uid() -> u32 {
    own_uid(&std::fs::read_to_string("/proc/self/status").unwrap()).expect("own user id")
}

fn config(bus: &TestBus, programs: &[String], open_timeout: Duration) -> Config {
    Config {
        bus: Bus::Address(bus.address.clone()),
        name: BUS_NAME.to_owned(),
        programs: programs.to_vec(),
        own_uid: uid(),
        open_timeout,
    }
}

const LONG: Duration = Duration::from_secs(120);

fn start(bus: &TestBus, programs: &[String]) -> (Rc<Helper>, Rc<Recorder>) {
    let recorder = Rc::new(Recorder::default());
    let handler: Rc<dyn Handler> = recorder.clone();
    let helper =
        run(Helper::start(config(bus, programs, LONG), handler)).expect("the helper starts");
    (Rc::new(helper), recorder)
}

/// Runs a caller program that calls every method, while the helper (on this
/// thread's main context) answers. Returns `(method, result)` lines.
fn run_child(bus: &TestBus, mode: &str) -> Vec<(String, String)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vexos-ai-helper-fakecaller"))
        .args([bus.address.as_str(), mode])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let context = MainContext::default();
    let began = Instant::now();
    loop {
        while context.iteration(false) {}
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if began.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("the caller program did not finish");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    out.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(m, r)| (m.to_owned(), r.to_owned()))
        .collect()
}

fn all(result: &[(String, String)]) -> Vec<&str> {
    result.iter().map(|(_, r)| r.as_str()).collect()
}

fn typed_calls() -> Vec<Call> {
    vec![
        Call::Open,
        Call::Close,
        Call::Edited("hi".into()),
        Call::Send {
            text: "hi".into(),
            via: Via::Button,
        },
        Call::Cancel,
        Call::CheckUsage,
        Call::DraftChoice(Placement::Keep),
        Call::DraftDiscard,
        Call::Decide {
            generation: 1,
            request: 2,
            choice: Choice::DenyOnce,
        },
        Call::Shown {
            generation: 1,
            request: 2,
        },
    ]
}

// The caller check.

fn a_stranger_is_refused_for_every_method() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        // the stranger runs as this user and from this program: only the owner
        // rule can refuse it
        let stranger = connect(&bus.address).await;
        for (method, params) in calls::calls() {
            assert_eq!(
                call(&stranger, method, params).await,
                Err(ACCESS_DENIED.to_owned()),
                "{method}"
            );
        }
    });
    assert!(recorder.calls.borrow().is_empty());
}

fn the_shell_passes_the_gate_for_every_method_and_reaches_the_handler() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        for (method, params) in calls::calls() {
            assert_eq!(
                call(&shell, method, params).await,
                Err(NOT_READY.to_owned()),
                "{method}"
            );
        }
    });
    assert_eq!(*recorder.calls.borrow(), typed_calls());
    let names: Vec<&str> = recorder.calls.borrow().iter().map(Call::name).collect();
    assert_eq!(names, METHODS);
}

fn the_shell_running_another_program_is_refused_for_every_method() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &["some-other-program".to_owned()]);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        for (method, params) in calls::calls() {
            assert_eq!(
                call(&shell, method, params).await,
                Err(ACCESS_DENIED.to_owned()),
                "{method}"
            );
        }
    });
    assert!(recorder.calls.borrow().is_empty());
}

fn a_helper_that_accepts_no_program_refuses_everyone() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[]);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        assert_eq!(
            call(&shell, "Open", None).await,
            Err(ACCESS_DENIED.to_owned())
        );
    });
    assert!(recorder.calls.borrow().is_empty());
}

fn a_child_process_that_is_not_the_shell_is_refused_for_every_method() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program(), child_program()]);
    // an in-process Shell owns the name while the child, a stranger, calls
    let shell = run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        shell
    });
    let result = run_child(&bus, "stranger");
    drop(shell);
    assert_eq!(result.len(), 10, "{result:?}");
    assert!(
        all(&result).iter().all(|r| *r == ACCESS_DENIED),
        "{result:?}"
    );
    assert!(recorder.calls.borrow().is_empty());
}

fn the_shell_name_in_a_process_running_the_wrong_program_is_refused() {
    // The child owns org.gnome.Shell, so the owner rule passes; its program is
    // not in the list, so only the program rule can refuse it.
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    let result = run_child(&bus, "shell");
    assert_eq!(result.len(), 10, "{result:?}");
    assert!(
        all(&result).iter().all(|r| *r == ACCESS_DENIED),
        "{result:?}"
    );
    assert!(recorder.calls.borrow().is_empty());
}

fn the_same_child_passes_once_its_program_is_in_the_list() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[child_program()]);
    let result = run_child(&bus, "shell");
    assert_eq!(result.len(), 10, "{result:?}");
    assert!(all(&result).iter().all(|r| *r == NOT_READY), "{result:?}");
    assert_eq!(*recorder.calls.borrow(), typed_calls());
}

fn nobody_owning_the_shell_name_refuses_everything_and_the_owner_is_followed() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    run(async {
        let first = connect(&bus.address).await;
        let second = connect(&bus.address).await;
        // no owner yet
        assert_eq!(
            call(&first, "Open", None).await,
            Err(ACCESS_DENIED.to_owned())
        );
        // the first becomes the Shell
        own_name(&first, SHELL_NAME).await;
        assert_eq!(call(&first, "Open", None).await, Err(NOT_READY.to_owned()));
        assert_eq!(
            call(&second, "Open", None).await,
            Err(ACCESS_DENIED.to_owned())
        );
        // the Shell's name is released: refused again
        release_name(&first, SHELL_NAME).await;
        assert_eq!(
            call(&first, "Open", None).await,
            Err(ACCESS_DENIED.to_owned())
        );
        // another takes it over
        own_name(&second, SHELL_NAME).await;
        assert_eq!(
            call(&first, "Open", None).await,
            Err(ACCESS_DENIED.to_owned())
        );
        assert_eq!(call(&second, "Open", None).await, Err(NOT_READY.to_owned()));
    });
    assert_eq!(*recorder.calls.borrow(), [Call::Open, Call::Open]);
}

fn a_shell_that_owned_its_name_before_the_helper_started_is_found() {
    let bus = TestBus::start();
    let shell = run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        shell
    });
    let (_helper, recorder) = start(&bus, &[this_program()]);
    run(async {
        assert_eq!(call(&shell, "Open", None).await, Err(NOT_READY.to_owned()));
    });
    assert_eq!(*recorder.calls.borrow(), [Call::Open]);
}

// What a call may carry.

fn wrong_arguments_never_reach_the_handler() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    let at_limit = "a".repeat(60_000);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        let too_long = "a".repeat(60_001);
        for (method, params, error) in [
            ("Edited", (too_long.as_str(),).to_variant(), TOO_LONG),
            ("Send", (too_long.as_str(), "button").to_variant(), TOO_LONG),
            ("Send", ("hi", "click").to_variant(), INVALID),
            ("Send", ("hi", "Button").to_variant(), INVALID),
            ("DraftChoice", ("cancel",).to_variant(), INVALID),
            ("DraftChoice", ("",).to_variant(), INVALID),
            (
                "Decide",
                (1u64, 2u64, "approve-always").to_variant(),
                INVALID,
            ),
            ("Decide", (1u64, 2u64, "approve").to_variant(), INVALID),
            ("Decide", (1u64, 2u64, "").to_variant(), INVALID),
        ] {
            assert_eq!(
                call(&shell, method, Some(params.clone())).await,
                Err(error.to_owned()),
                "{method} {params}"
            );
        }
        // wrong types and counts are refused by the bus layer itself
        for (method, params) in [
            ("Edited", Some((1u64,).to_variant())),
            ("Edited", None),
            ("Open", Some(("x",).to_variant())),
            ("Decide", Some(("deny-once", 1u64, 2u64).to_variant())),
            ("Shown", Some((1u64,).to_variant())),
        ] {
            let got = call(&shell, method, params).await;
            assert!(
                got.is_err() && got != Err(NOT_READY.to_owned()),
                "{method}: {got:?}"
            );
        }
        // exactly at the limit passes, whole
        assert_eq!(
            call(&shell, "Edited", Some((at_limit.as_str(),).to_variant())).await,
            Err(NOT_READY.to_owned())
        );
    });
    assert_eq!(*recorder.calls.borrow(), [Call::Edited(at_limit)]);
}

fn an_error_never_carries_what_the_caller_sent() {
    let bus = TestBus::start();
    let (_helper, _recorder) = start(&bus, &[this_program()]);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        let stranger = connect(&bus.address).await;
        let secret = "SECRET-TEXT-".repeat(6000);
        let refused = call_at(
            &stranger,
            OBJECT_PATH,
            INTERFACE,
            "Edited",
            Some(("SECRET-TEXT",).to_variant()),
        )
        .await
        .unwrap_err();
        assert!(!refused.to_string().contains("SECRET"), "{refused}");
        let long = call_at(
            &shell,
            OBJECT_PATH,
            INTERFACE,
            "Edited",
            Some((secret.as_str(),).to_variant()),
        )
        .await
        .unwrap_err();
        assert_eq!(error_name(&long), TOO_LONG);
        assert!(!long.to_string().contains("SECRET"), "{long}");
        let invalid = call_at(
            &shell,
            OBJECT_PATH,
            INTERFACE,
            "Send",
            Some(("SECRET-TEXT", "SECRET-VIA").to_variant()),
        )
        .await
        .unwrap_err();
        assert!(!invalid.to_string().contains("SECRET"), "{invalid}");
    });
}

// The surface.

fn method_names(xml: &str) -> Vec<String> {
    xml.split("<method name=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap().to_owned())
        .collect()
}

fn the_object_has_exactly_the_listed_methods_and_nothing_else() {
    assert_eq!(method_names(INTERFACE_XML), METHODS);
    assert_eq!(INTERFACE_XML.matches("<interface ").count(), 1);
    assert!(!INTERFACE_XML.contains("<property"));
    assert!(!INTERFACE_XML.contains("<signal"));
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    run(async {
        let stranger = connect(&bus.address).await;
        // GDBus itself answers Introspect, for anyone: it shows no more than the file
        let reply = call_at(
            &stranger,
            OBJECT_PATH,
            "org.freedesktop.DBus.Introspectable",
            "Introspect",
            None,
        )
        .await
        .expect("introspect");
        let xml = reply.child_value(0).get::<String>().unwrap();
        let ours: Vec<String> = method_names(&xml)
            .into_iter()
            .filter(|m| METHODS.contains(&m.as_str()))
            .collect();
        assert_eq!(ours, METHODS);
        // our interface has no properties and no signals (GDBus adds its own
        // standard interfaces, which are not ours)
        let start = xml
            .find(&format!("<interface name=\"{INTERFACE}\">"))
            .expect("our interface is introspectable");
        let ours_xml = &xml[start..start + xml[start..].find("</interface>").unwrap()];
        assert!(!ours_xml.contains("<property"), "{ours_xml}");
        assert!(!ours_xml.contains("<signal"), "{ours_xml}");
        // no properties are exposed
        let all = call_at(
            &stranger,
            OBJECT_PATH,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some((INTERFACE,).to_variant()),
        )
        .await;
        if let Ok(all) = all {
            assert_eq!(all.child_value(0).n_children(), 0, "{all}");
        }
        assert!(
            call_at(
                &stranger,
                OBJECT_PATH,
                "org.freedesktop.DBus.Properties",
                "Get",
                Some((INTERFACE, "Anything").to_variant()),
            )
            .await
            .is_err()
        );
    });
    assert!(recorder.calls.borrow().is_empty());
}

fn unknown_methods_objects_and_interfaces_are_refused_even_for_the_shell() {
    let bus = TestBus::start();
    let (_helper, recorder) = start(&bus, &[this_program()]);
    run(async {
        let shell = connect(&bus.address).await;
        own_name(&shell, SHELL_NAME).await;
        for method in ["Frobnicate", "open", "Eval", "Quit", "SetMode"] {
            assert!(call(&shell, method, None).await.is_err(), "{method}");
        }
        assert!(
            call_at(
                &shell,
                "/io/github/victorytek/Other",
                INTERFACE,
                "Open",
                None
            )
            .await
            .is_err()
        );
        assert!(
            call_at(&shell, OBJECT_PATH, "org.example.Other", "Open", None)
                .await
                .is_err()
        );
    });
    assert!(recorder.calls.borrow().is_empty());
}

// Starting and stopping.

fn the_helper_exits_when_no_open_comes() {
    let bus = TestBus::start();
    let handler: Rc<dyn Handler> = Rc::new(Recorder::default());
    let helper = run(Helper::start(
        config(&bus, &[this_program()], Duration::from_millis(300)),
        handler,
    ))
    .expect("starts");
    let began = Instant::now();
    assert_eq!(helper.run(), Exit::NoOpen);
    assert!(
        began.elapsed() >= Duration::from_millis(250),
        "{:?}",
        began.elapsed()
    );
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
}

fn an_accepted_open_keeps_the_helper_running() {
    let bus = TestBus::start();
    let handler: Rc<dyn Handler> = Rc::new(Recorder::default());
    let helper = Rc::new(
        run(Helper::start(
            config(&bus, &[this_program()], Duration::from_millis(400)),
            handler,
        ))
        .expect("starts"),
    );
    let address = bus.address.clone();
    let answer = Rc::new(RefCell::new(None));
    let slot = answer.clone();
    MainContext::default().spawn_local(async move {
        let shell = connect(&address).await;
        own_name(&shell, SHELL_NAME).await;
        // the gate passes it (the handler says NotReady, but it was accepted)
        *slot.borrow_mut() = Some(call(&shell, "Open", None).await);
        glib::timeout_future(Duration::from_millis(1500)).await;
        drop(shell);
    });
    let quitter = helper.clone();
    glib::timeout_add_local_once(Duration::from_millis(1200), move || quitter.quit());
    assert_eq!(helper.run(), Exit::Quit);
    assert_eq!(*answer.borrow(), Some(Err(NOT_READY.to_owned())));
}

fn a_refused_open_does_not_keep_the_helper_running() {
    let bus = TestBus::start();
    let handler: Rc<dyn Handler> = Rc::new(Recorder::default());
    let helper = Rc::new(
        run(Helper::start(
            config(&bus, &[this_program()], Duration::from_millis(400)),
            handler,
        ))
        .expect("starts"),
    );
    let address = bus.address.clone();
    let answer = Rc::new(RefCell::new(None));
    let slot = answer.clone();
    MainContext::default().spawn_local(async move {
        let shell = connect(&address).await;
        own_name(&shell, SHELL_NAME).await;
        let stranger = connect(&address).await;
        *slot.borrow_mut() = Some(call(&stranger, "Open", None).await);
        glib::timeout_future(Duration::from_secs(2)).await;
        drop((shell, stranger));
    });
    let quitter = helper.clone();
    glib::timeout_add_local_once(Duration::from_secs(6), move || quitter.quit());
    assert_eq!(helper.run(), Exit::NoOpen);
    assert_eq!(*answer.borrow(), Some(Err(ACCESS_DENIED.to_owned())));
}

fn a_name_that_is_already_owned_is_not_taken_over() {
    let bus = TestBus::start();
    run(async {
        let squatter = connect(&bus.address).await;
        own_name(&squatter, BUS_NAME).await;
        let handler: Rc<dyn Handler> = Rc::new(Recorder::default());
        let started = Helper::start(config(&bus, &[this_program()], LONG), handler).await;
        assert!(
            matches!(started, Err(StartError::NameTaken)),
            "{:?}",
            started.err()
        );
        // and the squatter still owns it
        let owner = squatter
            .call_future(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                Some(&(BUS_NAME,).to_variant()),
                None,
                DBusCallFlags::NONE,
                5000,
            )
            .await
            .expect("GetNameOwner");
        assert_eq!(
            owner.child_value(0).get::<String>(),
            squatter.unique_name().map(|n| n.to_string())
        );
    });
}

fn dropping_the_helper_releases_its_name() {
    let bus = TestBus::start();
    let (helper, _recorder) = start(&bus, &[this_program()]);
    let client = run(connect(&bus.address));
    let owned = |client: DBusConnection| async move {
        client
            .call_future(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&(BUS_NAME,).to_variant()),
                None,
                DBusCallFlags::NONE,
                5000,
            )
            .await
            .expect("NameHasOwner")
            .get::<(bool,)>()
            .map(|(owned,)| owned)
            .unwrap_or(true)
    };
    assert!(run(owned(client.clone())));
    drop(helper);
    // the connection closes at once, but the bus tells us a moment later
    let began = Instant::now();
    while run(owned(client.clone())) {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the helper's name was not released"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn the_helper_exits_when_the_bus_goes_away() {
    let mut bus = TestBus::start();
    let handler: Rc<dyn Handler> = Rc::new(Recorder::default());
    let helper = Rc::new(
        run(Helper::start(
            config(&bus, &[this_program()], LONG),
            handler,
        ))
        .expect("starts"),
    );
    bus.stop();
    let quitter = helper.clone();
    glib::timeout_add_local_once(Duration::from_secs(10), move || quitter.quit());
    assert_eq!(helper.run(), Exit::NameLost);
}

macro_rules! scenarios {
    ($($name:ident),* $(,)?) => {
        #[test]
        fn the_helper_on_a_private_bus() {
            let mut failed = Vec::new();
            $(
                eprintln!("scenario: {}", stringify!($name));
                if std::panic::catch_unwind($name).is_err() {
                    failed.push(stringify!($name));
                }
            )*
            assert!(failed.is_empty(), "failed scenarios: {failed:?}");
        }
    };
}

scenarios!(
    a_stranger_is_refused_for_every_method,
    the_shell_passes_the_gate_for_every_method_and_reaches_the_handler,
    the_shell_running_another_program_is_refused_for_every_method,
    a_helper_that_accepts_no_program_refuses_everyone,
    a_child_process_that_is_not_the_shell_is_refused_for_every_method,
    the_shell_name_in_a_process_running_the_wrong_program_is_refused,
    the_same_child_passes_once_its_program_is_in_the_list,
    nobody_owning_the_shell_name_refuses_everything_and_the_owner_is_followed,
    a_shell_that_owned_its_name_before_the_helper_started_is_found,
    wrong_arguments_never_reach_the_handler,
    an_error_never_carries_what_the_caller_sent,
    the_object_has_exactly_the_listed_methods_and_nothing_else,
    unknown_methods_objects_and_interfaces_are_refused_even_for_the_shell,
    the_helper_exits_when_no_open_comes,
    an_accepted_open_keeps_the_helper_running,
    a_refused_open_does_not_keep_the_helper_running,
    a_name_that_is_already_owned_is_not_taken_over,
    dropping_the_helper_releases_its_name,
    the_helper_exits_when_the_bus_goes_away,
);
