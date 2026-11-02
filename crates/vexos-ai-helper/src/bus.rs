//! The helper on the session bus: it owns one well-known name, serves one
//! object, and answers a call only after the gate has passed it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use gio::prelude::*;
use gio::{
    BusType, DBusCallFlags, DBusConnection, DBusConnectionFlags, DBusMethodInvocation,
    DBusNodeInfo, DBusSignalFlags, SignalSubscription, UnixFDList,
};
use glib::prelude::ToVariant;
use glib::variant::Handle;
use glib::{MainLoop, SourceId, Variant, VariantDict, VariantTy};

use crate::gate::{ArgError, Args, Call, Gate, Identity, LookupFailure};
use crate::peers::{Credentials, identify};
use crate::rate::Limiter;

pub const BUS_NAME: &str = "io.github.victorytek.VexosAssistant.Helper";
pub const OBJECT_PATH: &str = "/io/github/victorytek/VexosAssistant/Helper";
pub const INTERFACE: &str = "io.github.victorytek.VexosAssistant.Helper1";
pub const ERROR_PREFIX: &str = "io.github.victorytek.VexosAssistant.Error.";
pub const SHELL_NAME: &str = "org.gnome.Shell";

/// The introspection data: the only methods the object has.
pub const INTERFACE_XML: &str = include_str!("../interface.xml");

const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
const UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
const DAEMON: &str = "org.freedesktop.DBus";
const DAEMON_PATH: &str = "/org/freedesktop/DBus";

pub enum Bus {
    Session,
    Address(String),
}

pub struct Config {
    pub bus: Bus,
    pub name: String,
    /// File names of the programs the GNOME Shell may run as.
    pub programs: Vec<String>,
    pub own_uid: u32,
    /// How long the helper waits for an accepted `Open` before it exits.
    pub open_timeout: Duration,
    /// The time the rate limits are counted in.
    pub clock: Rc<dyn Fn() -> Instant>,
}

/// What an accepted call answers with. No call returns a value yet.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Done,
    /// The reply is sent and flushed, then the helper exits.
    DoneThenExit,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HandlerError {
    NotReady,
}

/// Receives a call that passed the gate.
pub trait Handler {
    fn handle(&self, call: Call) -> Result<Reply, HandlerError>;

    /// The owner of `org.gnome.Shell` went away or changed. Ends the session
    /// (denying what is pending) and returns `true`, and then the helper exits;
    /// with no session it returns `false` and the helper carries on.
    fn shell_gone(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// No accepted `Open` came in time.
    NoOpen,
    /// The name was lost, or the bus went away.
    NameLost,
    /// Asked to stop: the session was closed.
    Quit,
    /// The GNOME Shell went away while a session was open.
    ShellGone,
}

#[derive(Debug)]
pub enum StartError {
    Connect(String),
    Interface(String),
    NameTaken,
}

/// Ends the helper, once, from any thread (GIO may report a closed connection
/// from another thread).
struct Ender {
    /// 0 while running, else the `Exit` as a number.
    code: AtomicU8,
    main_loop: MainLoop,
}

impl Ender {
    fn finish(&self, exit: Exit) {
        let code = match exit {
            Exit::NoOpen => 1,
            Exit::NameLost => 2,
            Exit::Quit => 3,
            Exit::ShellGone => 4,
        };
        if self
            .code
            .compare_exchange(0, code, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.main_loop.quit();
        }
    }

    fn exit(&self) -> Option<Exit> {
        match self.code.load(Ordering::SeqCst) {
            1 => Some(Exit::NoOpen),
            2 => Some(Exit::NameLost),
            3 => Some(Exit::Quit),
            4 => Some(Exit::ShellGone),
            _ => None,
        }
    }
}

struct State {
    gate: Gate,
    handler: Rc<dyn Handler>,
    shell_owner: RefCell<Option<String>>,
    /// The owner was learned from a signal: a later `GetNameOwner` reply is older.
    owner_from_signal: Cell<bool>,
    ender: Arc<Ender>,
    timer: RefCell<Option<SourceId>>,
    limiter: RefCell<Limiter>,
    clock: Rc<dyn Fn() -> Instant>,
}

/// Dropping it drops the connection, which releases the name.
pub struct Helper {
    state: Rc<State>,
    _connection: DBusConnection,
    _subscriptions: Vec<SignalSubscription>,
}

impl Helper {
    /// Connects, serves the object, tracks the Shell's name, and owns the
    /// helper's name. Returns once the name is owned, or why it cannot be.
    pub async fn start(config: Config, handler: Rc<dyn Handler>) -> Result<Helper, StartError> {
        let flags = DBusConnectionFlags::AUTHENTICATION_CLIENT
            | DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
        let connection = match &config.bus {
            Bus::Session => gio::bus_get_future(BusType::Session).await,
            Bus::Address(address) => DBusConnection::for_address_future(address, flags, None).await,
        }
        .map_err(|e| StartError::Connect(e.to_string()))?;
        // The helper ends itself, and says why: it does not let GIO exit.
        connection.set_exit_on_close(false);
        let programs: Vec<&str> = config.programs.iter().map(String::as_str).collect();
        let state = Rc::new(State {
            gate: Gate::new(config.own_uid, &programs),
            handler,
            shell_owner: RefCell::new(None),
            owner_from_signal: Cell::new(false),
            ender: Arc::new(Ender {
                code: AtomicU8::new(0),
                main_loop: MainLoop::new(None, false),
            }),
            timer: RefCell::new(None),
            limiter: RefCell::new(Limiter::default()),
            clock: config.clock.clone(),
        });
        {
            let ender = state.ender.clone();
            connection.connect_closed(move |_, _, _| ender.finish(Exit::NameLost));
        }

        // Who owns org.gnome.Shell: the signal first, so no change is missed
        // between the subscription and the first answer.
        let owner_subscription = {
            let state = state.clone();
            connection.subscribe_to_signal(
                Some(DAEMON),
                Some(DAEMON),
                Some("NameOwnerChanged"),
                Some(DAEMON_PATH),
                Some(SHELL_NAME),
                DBusSignalFlags::NONE,
                move |signal| {
                    if let Some((name, old, new)) =
                        signal.parameters.get::<(String, String, String)>()
                        && name == SHELL_NAME
                    {
                        state.owner_from_signal.set(true);
                        *state.shell_owner.borrow_mut() = (!new.is_empty()).then_some(new);
                        // A Shell that had the name and no longer does: the
                        // panel is gone. A session ends, and then so does the helper.
                        if !old.is_empty() && state.handler.shell_gone() {
                            state.finish(Exit::ShellGone);
                        }
                    }
                },
            )
        };
        if let Ok(reply) = connection
            .call_future(
                Some(DAEMON),
                DAEMON_PATH,
                DAEMON,
                "GetNameOwner",
                Some(&(SHELL_NAME,).to_variant()),
                None,
                DBusCallFlags::NONE,
                5000,
            )
            .await
            && !state.owner_from_signal.get()
        {
            *state.shell_owner.borrow_mut() = reply.child_value(0).get::<String>();
        }

        // The object, before the name: nothing can call it unserved.
        let node = DBusNodeInfo::for_xml(INTERFACE_XML)
            .map_err(|e| StartError::Interface(e.to_string()))?;
        let info = node
            .lookup_interface(INTERFACE)
            .ok_or_else(|| StartError::Interface("interface missing".to_owned()))?;
        {
            let state = state.clone();
            connection
                .register_object(OBJECT_PATH, &info)
                .method_call(move |conn, sender, _, _, method, params, invocation| {
                    let state = state.clone();
                    let sender = sender.map(str::to_owned);
                    let method = method.to_owned();
                    glib::spawn_future_local(async move {
                        state.serve(conn, sender, method, params, invocation).await;
                    });
                })
                .build()
                .map_err(|e| StartError::Interface(e.to_string()))?
        };

        // The name: never queued, never replaced, so only this helper's own
        // connection closing can end its ownership (handled above). (Not
        // `bus_own_name_on_connection`: when the bus goes away it calls back with
        // no connection, which the Rust binding unwraps, and the process aborts.)
        // 4 = do not queue: the reply is 1 (owner now) or 3 (taken by another).
        let reply = connection
            .call_future(
                Some(DAEMON),
                DAEMON_PATH,
                DAEMON,
                "RequestName",
                Some(&(config.name.as_str(), 4u32).to_variant()),
                None,
                DBusCallFlags::NONE,
                5000,
            )
            .await
            .map_err(|e| StartError::Connect(e.to_string()))?;
        if reply.get::<(u32,)>() != Some((1,)) {
            return Err(StartError::NameTaken);
        }

        // Nothing resident for a stray activation: wait for one accepted Open.
        {
            let for_timer = state.clone();
            let id = glib::timeout_add_local_once(config.open_timeout, move || {
                for_timer.timer.borrow_mut().take();
                for_timer.finish(Exit::NoOpen);
            });
            *state.timer.borrow_mut() = Some(id);
        }
        Ok(Helper {
            state,
            _connection: connection,
            _subscriptions: vec![owner_subscription],
        })
    }

    /// Serves until the helper ends, and says why.
    pub fn run(&self) -> Exit {
        if self.state.ender.exit().is_none() {
            self.state.ender.main_loop.run();
        }
        self.state.ender.exit().unwrap_or(Exit::Quit)
    }

    pub fn quit(&self) {
        self.state.finish(Exit::Quit);
    }
}

impl State {
    fn finish(&self, exit: Exit) {
        self.ender.finish(exit);
    }

    fn cancel_timer(&self) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
    }

    /// One call: the caller check, then the arguments, then the handler. Each
    /// refusal is a fixed error; nothing the caller sent is in it or in the log.
    async fn serve(
        &self,
        conn: DBusConnection,
        sender: Option<String>,
        method: String,
        params: Variant,
        invocation: DBusMethodInvocation,
    ) {
        // Only the Shell's connection is looked up: a stranger costs nothing.
        let owner = self.shell_owner.borrow().clone();
        let identity = match (&sender, &owner) {
            (Some(s), Some(o)) if !s.is_empty() && s == o => lookup_identity(&conn, s).await,
            _ => Err(LookupFailure::Credentials),
        };
        // The owner can change while the lookup runs: the gate sees the current one.
        let owner = self.shell_owner.borrow().clone();
        if let Err(why) = self
            .gate
            .check_caller(sender.as_deref(), owner.as_deref(), &identity)
        {
            eprintln!("vexos-ai-helper: refused {method}: {why:?}");
            invocation.return_dbus_error(ACCESS_DENIED, "The caller is not allowed.");
            return;
        }

        let mut args = Args::default();
        for i in 0..params.n_children() {
            let child = params.child_value(i);
            if let Some(text) = child.str() {
                args.strings.push(text.to_owned());
            } else if let Some(number) = child.get::<u64>() {
                args.numbers.push(number);
            } else {
                invocation.return_dbus_error(&error("InvalidArgument"), "The arguments are wrong.");
                return;
            }
        }
        let call = match Call::parse(&method, &args) {
            Ok(call) => call,
            Err(ArgError::UnknownMethod) => {
                invocation.return_dbus_error(UNKNOWN_METHOD, "There is no such method.");
                return;
            }
            Err(ArgError::TooLong) => {
                // An over-long `Edited` leaves the input holding text the
                // handler never saw; it must know. A `Send` leaves it as it was.
                if method == "Edited" {
                    let _ = self.handler.handle(Call::EditLost);
                }
                invocation.return_dbus_error(&error("TooLong"), "The text is too long.");
                return;
            }
            Err(ArgError::InvalidArgument) => {
                invocation.return_dbus_error(&error("InvalidArgument"), "The arguments are wrong.");
                return;
            }
        };
        if matches!(call, Call::Open) {
            self.cancel_timer();
        }
        let allowed = self.limiter.borrow_mut().allow(call.name(), (self.clock)());
        if !allowed {
            if matches!(call, Call::Edited(_)) {
                let _ = self.handler.handle(Call::EditLost);
            }
            invocation.return_dbus_error(&error("TooFast"), "Too many calls.");
            return;
        }
        match self.handler.handle(call) {
            Ok(Reply::Done) => invocation.return_value(Some(&().to_variant())),
            Ok(Reply::DoneThenExit) => {
                invocation.return_value(Some(&().to_variant()));
                // The reply must be on the wire before the process ends.
                let _ = conn.flush_future().await;
                self.finish(Exit::Quit);
            }
            Err(HandlerError::NotReady) => {
                invocation.return_dbus_error(&error("NotReady"), "The helper is not ready.");
            }
        }
    }
}

fn error(name: &str) -> String {
    format!("{ERROR_PREFIX}{name}")
}

/// What the bus says about the process behind `name`, checked against `/proc`.
async fn lookup_identity(conn: &DBusConnection, name: &str) -> Result<Identity, LookupFailure> {
    let (body, fds) = conn
        .call_with_unix_fd_list_future(
            Some(DAEMON),
            DAEMON_PATH,
            DAEMON,
            "GetConnectionCredentials",
            Some(&(name,).to_variant()),
            VariantTy::new("(a{sv})").ok(),
            DBusCallFlags::NONE,
            5000,
            None::<&UnixFDList>,
        )
        .await
        .map_err(|_| LookupFailure::Credentials)?;
    let dict = VariantDict::new(Some(&body.child_value(0)));
    let number = |key: &str| dict.lookup::<u32>(key).ok().flatten();
    let pidfd = dict
        .lookup_value("ProcessFD", None)
        .and_then(|value| value.get::<Handle>())
        .and_then(|handle| usize::try_from(handle.0).ok())
        .and_then(|index| fds.as_ref()?.peek_fds().get(index).copied());
    let creds = Credentials {
        uid: number("UnixUserID"),
        pid: number("ProcessID"),
        pidfd,
    };
    let fd = creds.pidfd.unwrap_or(-1);
    // `fds` stays alive to the end of this function: the descriptor is open.
    identify(
        &creds,
        &mut || std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).ok(),
        &|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_names_are_under_the_helper_prefix() {
        assert_eq!(
            error("NotReady"),
            "io.github.victorytek.VexosAssistant.Error.NotReady"
        );
        assert_eq!(
            error("TooFast"),
            "io.github.victorytek.VexosAssistant.Error.TooFast"
        );
    }
}
