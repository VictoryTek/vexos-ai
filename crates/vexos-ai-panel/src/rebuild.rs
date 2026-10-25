//! "Run just rebuild": the user's action, kept apart from the agent session.
//! The command comes from `RebuildCommand` (fixed argv, fixed directory), runs
//! in its own window after a confirmation, and its terminal is never read
//! into or written from the conversation.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use vexos_ai_viewmodel::{Rebuild, RebuildCommand};
use vte::prelude::*;

use crate::Ui;

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
            status.set_label(&finished(&ui));
            ui.refresh();
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
}

fn finished(ui: &Ui) -> String {
    match &*ui.rebuild.borrow() {
        Rebuild::Finished(how) => {
            format!("just rebuild {how}. This output stays here; it is not sent to the assistant.")
        }
        _ => String::new(),
    }
}
