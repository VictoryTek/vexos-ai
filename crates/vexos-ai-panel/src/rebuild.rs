//! "Run just rebuild": the user's action, kept apart from the agent session.
//! The command comes from `RebuildCommand` (fixed argv, fixed directory), runs
//! in its own window after a confirmation, and nothing writes into its
//! terminal. Its output is read only when the user clicks the failure
//! notification, into a draft they review before anything is sent.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use vexos_ai_viewmodel::{Log, Missing, Rebuild, RebuildCommand, excerpt};
use vte::prelude::*;

use crate::{Ui, notify};

/// The last run's window, kept (hidden once closed) until the next run, so
/// its output can still be read when the notification is clicked.
pub(crate) struct RebuildView {
    window: adw::Window,
    terminal: vte::Terminal,
    wait_status: Cell<Option<i32>>,
}

/// The button was clicked: confirm first.
pub(crate) fn ask(ui: &Rc<Ui>) {
    if !ui.rebuild.borrow_mut().request() {
        return;
    }
    ui.refresh();
    let dialog = adw::AlertDialog::new(
        Some("Run just rebuild?"),
        Some(
            "This runs “just rebuild” in /etc/nixos: it builds this system's \
             configuration and applies it. sudo asks for your password in the \
             terminal that opens. The assistant cannot see or type into it.",
        ),
    );
    dialog.add_responses(&[("cancel", "_Cancel"), ("run", "_Run")]);
    dialog.set_response_appearance("run", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let response = dialog.choose_future(Some(&ui.window)).await;
        let command = if response == "run" {
            ui.rebuild.borrow_mut().confirm()
        } else {
            ui.rebuild.borrow_mut().dismiss();
            None
        };
        if let Some(command) = command {
            run(&ui, command);
        }
        ui.refresh();
    });
}

/// Spawns the command directly in a terminal: no shell, so once it exits
/// there is nothing left to type into, and the input is turned off too.
fn run(ui: &Rc<Ui>, command: RebuildCommand) {
    if let Some(old) = ui.rebuild_view.borrow_mut().take() {
        old.window.destroy();
    }
    let terminal = vte::Terminal::builder()
        .vexpand(true)
        .hexpand(true)
        .scrollback_lines(10_000)
        .build();
    let status = gtk::Label::builder()
        .label("Running just rebuild in /etc/nixos…")
        .xalign(0.0)
        .wrap(true)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&terminal);
    body.append(&status);
    let content = adw::ToolbarView::new();
    content.add_top_bar(&adw::HeaderBar::new());
    content.set_content(Some(&body));
    let window = adw::Window::builder()
        .title("System rebuild")
        .default_width(900)
        .default_height(560)
        .transient_for(&ui.window)
        .hide_on_close(true)
        .content(&content)
        .build();

    window.connect_close_request({
        let (ui, status) = (ui.clone(), status.clone());
        move |_| {
            if ui.rebuild.borrow().can_close() {
                glib::Propagation::Proceed
            } else {
                status.set_label("Still running. Close this window when it has finished.");
                glib::Propagation::Stop
            }
        }
    });
    terminal.connect_child_exited({
        let (ui, status) = (ui.clone(), status.clone());
        move |terminal, wait_status| {
            terminal.set_input_enabled(false);
            ui.rebuild.borrow_mut().exited(wait_status);
            if let Some(view) = ui.rebuild_view.borrow().as_ref() {
                view.wait_status.set(Some(wait_status));
            }
            status.set_label(&finished(&ui));
            ui.refresh();
            if wait_status != 0
                && let Some(app) = ui.window.application()
            {
                notify::rebuild_failed(&app, wait_status);
            }
        }
    });
    terminal.spawn_async(
        vte::PtyFlags::DEFAULT,
        Some(command.dir()),
        command.argv(),
        &[],
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        {
            let (ui, status, terminal) = (ui.clone(), status, terminal.clone());
            move |result| {
                if let Err(e) = result {
                    terminal.set_input_enabled(false);
                    ui.rebuild.borrow_mut().failed_to_start(e.message());
                    status.set_label(&finished(&ui));
                    ui.refresh();
                }
            }
        },
    );
    window.present();
    *ui.rebuild_view.borrow_mut() = Some(RebuildView {
        window,
        terminal,
        wait_status: Cell::new(None),
    });
}

/// The finished run's wait status and terminal text, read now. Gone once
/// another run starts or the panel closes.
pub(crate) fn output(ui: &Ui) -> (Option<i32>, Log) {
    let view = ui.rebuild_view.borrow();
    let Some(view) = view.as_ref() else {
        return (None, Log::Missing(Missing::OutputGone));
    };
    if !matches!(*ui.rebuild.borrow(), Rebuild::Finished(_)) {
        return (None, Log::Missing(Missing::OutputGone));
    }
    let t = &view.terminal;
    // Rows count from the oldest line still in the scrollback.
    let first = t.vadjustment().map_or(0.0, |a| a.lower()) as i64;
    let (last, _) = t.cursor_position();
    let (text, _) = t.text_range_format(vte::Format::Text, first as _, 0, last, t.column_count());
    let text = text.map(|t| t.to_string()).unwrap_or_default();
    (
        view.wait_status.get(),
        Log::Collected(excerpt(text.as_bytes())),
    )
}

fn finished(ui: &Ui) -> String {
    match &*ui.rebuild.borrow() {
        Rebuild::Finished(how) => {
            format!(
                "just rebuild {how}. This output is not sent to the assistant unless you send it \
                 yourself from a draft."
            )
        }
        _ => String::new(),
    }
}
