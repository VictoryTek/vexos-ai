//! Test-only: a separate process that calls every method of the helper, so
//! the tests see a caller whose process id and program differ from their own.
//! usage: fakecaller ADDRESS shell|stranger
//! Prints one line per method: `Method=ok` or `Method=<error name>`.

use gio::{Cancellable, DBusCallFlags, DBusConnection, DBusConnectionFlags};
use glib::prelude::ToVariant;
use vexos_ai_helper::bus::{BUS_NAME, INTERFACE, OBJECT_PATH, SHELL_NAME};

#[path = "calls.rs"]
mod calls;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (address, mode) = (&args[1], &args[2]);
    let conn = DBusConnection::for_address_sync(
        address,
        DBusConnectionFlags::AUTHENTICATION_CLIENT | DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        None::<&Cancellable>,
    )
    .expect("connect to the bus");
    if mode == "shell" {
        // 4 = do not queue: the reply is 1 (this process owns the name) or 3
        // (someone else does): only 1 is accepted, so a caller that says it is
        // the Shell really is.
        let reply = conn
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "RequestName",
                Some(&(SHELL_NAME, 4u32).to_variant()),
                None,
                DBusCallFlags::NONE,
                5000,
                None::<&Cancellable>,
            )
            .expect("RequestName");
        assert_eq!(
            reply.get::<(u32,)>(),
            Some((1,)),
            "not the owner of the Shell name"
        );
    }
    for (method, params) in calls::calls() {
        let reply = conn.call_sync(
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            method,
            params.as_ref(),
            None,
            DBusCallFlags::NONE,
            5000,
            None::<&Cancellable>,
        );
        match reply {
            Ok(_) => println!("{method}=ok"),
            Err(e) => println!(
                "{method}={}",
                gio::DBusError::remote_error(&e)
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| "local-error".to_owned())
            ),
        }
    }
}
