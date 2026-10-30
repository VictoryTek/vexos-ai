//! The VexOS assistant panel: a plain window over `vexos-ai-viewmodel`. It
//! renders what the view-model holds and forwards clicks to it; the rules
//! (what reaches the user, what counts as an answer) live in the core and the
//! view-model, which are tested headless.

mod notify;
#[cfg(test)]
mod scan_tests;
mod style;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use style::TagColors;
use vexos_ai_core::{Agent, Config, Decision, Error, Session};
use vexos_ai_viewmodel::{
    AGENT_FILE, Answer, ApprovalCard, Composer, Conversation, DiffLine, EXTENSION_BUS_NAME, Empty,
    Item, Offer, Outgoing, PANEL_SETTINGS_FILE, PanelSettings, Placement, Proposal, Sent, Startup,
    State, USAGE_LOCK, Via, WINDOW_MODE_KEY, WINDOW_SCHEMA, WindowMode, agent_label, docked_layout,
    markdown, mode_label, saved_agent, since_process_start, window_look,
};

const APP_ID: &str = "io.github.victorytek.VexosAssistant";
/// Where every session runs, as `vexos-ai` does in the terminal.
const WORKDIR: &str = "/etc/nixos";
/// The agent starts after the window's first frame, because its start takes
/// CPU the first frame needs; or after this long if no frame ever comes.
const START_DEADLINE: Duration = Duration::from_secs(3);

/// The panel's own settings, read once; the file is never written.
fn panel_settings() -> PanelSettings {
    static SETTINGS: OnceLock<PanelSettings> = OnceLock::new();
    *SETTINGS.get_or_init(|| {
        std::fs::read_to_string(glib::user_config_dir().join(PANEL_SETTINGS_FILE))
            .map(|text| PanelSettings::parse(&text))
            .unwrap_or_default()
    })
}

thread_local! {
    /// Start-up times of this process, taken by its first window. Display
    /// only: shown in the title tooltip and nowhere else.
    static STARTUP: Cell<Option<Startup>> = const {
        Cell::new(Some(Startup { before_main: None, main: None, gtk_ready: None, styled: None }))
    };
}

fn mark(set: impl FnOnce(&mut Startup)) {
    STARTUP.with(|cell| {
        if let Some(mut startup) = cell.get() {
            set(&mut startup);
            cell.set(Some(startup));
        }
    });
}

/// How long this process has been running (read only).
fn process_age() -> Option<Duration> {
    since_process_start(
        &std::fs::read_to_string("/proc/self/stat").ok()?,
        &std::fs::read_to_string("/proc/uptime").ok()?,
    )
}

/// The core is async on tokio; GTK owns the main thread.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("start the tokio runtime"))
}

/// The one window of this instance, while it is open (shown or hidden).
type Slot = Rc<RefCell<Option<Rc<Ui>>>>;

/// What the `toggle` action does, given whether the window is open and shown.
#[derive(Debug, PartialEq, Eq)]
enum Toggle {
    Open,
    Show,
    Hide,
}

fn toggle(window: Option<bool>) -> Toggle {
    match window {
        None => Toggle::Open,
        Some(false) => Toggle::Show,
        Some(true) => Toggle::Hide,
    }
}

fn main() -> glib::ExitCode {
    let age = process_age();
    mark(|s| {
        s.before_main = age;
        s.main = Some(Instant::now());
    });
    let app = adw::Application::builder().application_id(APP_ID).build();
    let slot = Slot::default();
    app.connect_startup(|_| {
        // GTK and libadwaita are initialised before this handler runs.
        mark(|s| s.gtk_ready = Some(Instant::now()));
        if let Some(display) = gdk::Display::default() {
            style::load(&display);
        }
        mark(|s| s.styled = Some(Instant::now()));
    });
    // Watchers run only in the primary instance.
    app.connect_startup(notify::start_watchers);
    app.connect_activate({
        let slot = slot.clone();
        move |app| window(app, &slot).window.present()
    });
    // For the Shell extension, over D-Bus (`org.freedesktop.Application`). It
    // takes no parameter, so a caller can show or hide the window and nothing
    // else. Hiding keeps the session and any pending approval as they are;
    // only closing the window ends the session (and denies what is pending).
    let toggle_action = gio::ActionEntry::builder("toggle")
        .activate({
            let slot = slot.clone();
            move |app: &adw::Application, _, _| {
                let shown = slot.borrow().as_ref().map(|ui| ui.window.is_visible());
                match toggle(shown) {
                    Toggle::Open | Toggle::Show => window(app, &slot).window.present(),
                    Toggle::Hide => window(app, &slot).window.set_visible(false),
                }
            }
        })
        .build();
    app.add_action_entries([toggle_action]);
    app.add_action_entries(notify::actions(&slot));
    app.run()
}

/// The open window, or a new one with a new session after a close.
fn window(app: &adw::Application, slot: &Slot) -> Rc<Ui> {
    if let Some(ui) = slot.borrow().as_ref() {
        return ui.clone();
    }
    let requested = Instant::now();
    let ui = Ui::build(app);
    let built = Instant::now();
    time_first_frame(&ui);
    // Runs after the session's own close handler.
    ui.window.connect_close_request({
        let slot = slot.clone();
        move |_| {
            slot.borrow_mut().take();
            glib::Propagation::Proceed
        }
    });
    *slot.borrow_mut() = Some(ui.clone());
    // Every way of opening the window starts the saved agent. Read-only; the
    // panel never writes the choice and has no switcher.
    ui.start(
        saved_agent(&glib::user_config_dir().join(AGENT_FILE)),
        requested,
        built,
    );
    ui
}

/// Records when the window is first mapped and when its first frame is
/// painted, for the launch timing, and then lets the agent start.
fn time_first_frame(ui: &Rc<Ui>) {
    let armed = Cell::new(true);
    let weak = Rc::downgrade(ui);
    ui.window.connect_map(move |window| {
        let note = |record: fn(&mut Conversation, Instant)| {
            if let Some(ui) = weak.upgrade() {
                ui.with_conversation(|c| {
                    record(c, Instant::now());
                    Vec::new()
                });
            }
        };
        note(Conversation::mapped);
        if !armed.replace(false) {
            return;
        }
        let Some(clock) = window.frame_clock() else {
            return;
        };
        let handler = Rc::new(Cell::new(None::<glib::SignalHandlerId>));
        let id = clock.connect_after_paint({
            let handler = handler.clone();
            let weak = weak.clone();
            move |clock| {
                if let Some(id) = handler.take() {
                    clock.disconnect(id);
                }
                if let Some(ui) = weak.upgrade() {
                    ui.with_conversation(|c| {
                        c.first_frame(Instant::now());
                        Vec::new()
                    });
                    ui.begin_start();
                }
            }
        });
        handler.set(Some(id));
    });
}

struct Ui {
    window: adw::ApplicationWindow,
    header: adw::HeaderBar,
    title: adw::WindowTitle,
    list: gtk::Box,
    /// The empty page or the list, with a short crossfade.
    stack: gtk::Stack,
    empty_title: gtk::Label,
    empty_hint: gtk::Label,
    empty_spinner: adw::Spinner,
    scroller: gtk::ScrolledWindow,
    rows: RefCell<Vec<gtk::Widget>>,
    input: gtk::TextView,
    send: gtk::Button,
    /// The usage line under the header. Its text is only ever the
    /// view-model's label, and a click sends the view-model's constant
    /// `/usage`; it has no other behaviour.
    usage: gtk::Button,
    /// The window is narrow: the usage line shows only the percent.
    narrow: Cell<bool>,
    /// Above the input while the launch `/usage` holds it (its note slides in).
    lock_reveal: gtk::Revealer,
    cancel: gtk::Button,
    /// Docked or floating, shown only with the Shell extension running. Its
    /// only action is writing `WindowMode::as_str()` to the shared setting.
    dock_toggle: gtk::Button,
    /// The extension's schema, when installed with the panel.
    mode_settings: Option<gio::Settings>,
    extension_running: Cell<bool>,
    watcher: RefCell<Option<Box<dyn FnOnce()>>>,
    /// The agent to start once the window has had its first frame.
    pending_start: Cell<Option<Agent>>,
    conversation: RefCell<Option<Conversation>>,
    session: RefCell<Option<Arc<Session>>>,
    /// What is in the input, and whether it is an unsent draft. The only
    /// source of a message to send.
    composer: RefCell<Composer>,
    /// Set while the input is filled from `composer`, so the buffer's own
    /// change signals are not taken for the user's edits.
    syncing: Cell<bool>,
    /// Above the input while the session starts: its text is the
    /// view-model's `starting_note`.
    starting_note: gtk::Label,
    /// Above the input while it holds a draft: review, Send, or Discard.
    banner: adw::Banner,
}

impl Ui {
    fn build(app: &adw::Application) -> Rc<Ui> {
        let title = adw::WindowTitle::new("VexOS Assistant", "starting…");
        let cancel = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text("Cancel the current turn")
            .sensitive(false)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&cancel);
        let dock_toggle = gtk::Button::builder()
            .visible(false)
            .focus_on_click(false)
            .build();
        dock_toggle.add_css_class("flat");
        header.pack_end(&dock_toggle);
        // Never `Settings::new`: it aborts when the schema is missing.
        let mode_settings = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(WINDOW_SCHEMA, true))
            .map(|schema| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None));

        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        // Multi-line, so a draft shows exactly what would be sent. Enter
        // sends, Shift+Enter starts a new line.
        let input = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .tooltip_text("Ask about this system… (Enter sends, Shift+Enter for a new line)")
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(12)
            .right_margin(12)
            .sensitive(false)
            .build();
        let input_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .propagate_natural_height(true)
            .max_content_height(200)
            .child(&input)
            .build();
        input_scroller.add_css_class("card");
        input_scroller.add_css_class("vexos-input");
        // Until the draft is edited, Enter does not send it; the Send button does.
        let banner = adw::Banner::builder()
            .title("Draft from a notification: review it, then click Send")
            .button_label("_Discard")
            .revealed(false)
            .build();
        let send = gtk::Button::builder()
            .label("Send")
            .sensitive(false)
            .build();
        send.add_css_class("suggested-action");
        send.add_css_class("vexos-send");
        // Fill the row: as tall as the input, whether it holds one line or many.
        send.set_valign(gtk::Align::Fill);
        let composer = gtk::Box::builder()
            .spacing(8)
            .margin_top(8)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        composer.append(&input_scroller);
        composer.append(&send);
        let lock_note = gtk::Label::builder()
            .label("Checking usage… The input opens when it finishes, or in 15 seconds.")
            .wrap(true)
            .build();
        lock_note.add_css_class("vexos-quiet");
        lock_note.add_css_class("caption");
        let lock_reveal = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(150)
            .child(&lock_note)
            .build();
        // The page before the first message: a spinner while the session
        // starts (the starting note stays above the input, visible whatever
        // the list shows), then which agent is ready. Plain labels only.
        let starting_note = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .visible(false)
            .build();
        starting_note.add_css_class("vexos-quiet");
        let empty_spinner = adw::Spinner::builder()
            .width_request(32)
            .height_request(32)
            .build();
        let empty_title = gtk::Label::builder().wrap(true).build();
        empty_title.add_css_class("title-3");
        let empty_hint = gtk::Label::new(Some("Ask about this system."));
        empty_hint.add_css_class("vexos-quiet");
        let empty = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        empty.add_css_class("vexos-empty");
        empty.append(&empty_spinner);
        empty.append(&empty_title);
        empty.append(&empty_hint);
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(150)
            .vexpand(true)
            .build();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroller, Some("list"));
        stack.set_visible_child_name("empty");
        let chat = gtk::Box::new(gtk::Orientation::Vertical, 0);
        chat.append(&stack);
        chat.append(&banner);
        chat.append(&starting_note);
        chat.append(&lock_reveal);
        chat.append(&composer);

        // The usage line: flat, under the header, and the only control that
        // shows the full report. Shown once a Claude session exists.
        let usage = gtk::Button::builder()
            .label("Usage: loading")
            .halign(gtk::Align::Center)
            .visible(false)
            .sensitive(false)
            .build();
        usage.add_css_class("flat");
        usage.add_css_class("caption");
        usage.add_css_class("numeric");
        usage.add_css_class("vexos-quiet");
        usage.add_css_class("vexos-usage");

        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.add_top_bar(&usage);
        content.set_content(Some(&chat));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("VexOS Assistant")
            .default_width(760)
            .default_height(820)
            // A breakpoint needs a minimum size.
            .width_request(360)
            .height_request(400)
            .content(&content)
            .build();

        let ui = Rc::new(Ui {
            window,
            header,
            title,
            list,
            stack,
            empty_title,
            empty_hint,
            empty_spinner,
            scroller,
            rows: RefCell::new(Vec::new()),
            input,
            send,
            usage,
            narrow: Cell::new(false),
            lock_reveal,
            cancel,
            dock_toggle,
            mode_settings,
            extension_running: Cell::new(false),
            watcher: RefCell::new(None),
            pending_start: Cell::new(None),
            conversation: RefCell::new(None),
            session: RefCell::new(None),
            composer: RefCell::new(Composer::default()),
            syncing: Cell::new(false),
            starting_note,
            banner,
        });

        ui.send.connect_clicked({
            let ui = ui.clone();
            move |_| ui.submit(Via::SendButton)
        });
        ui.usage.connect_clicked({
            let ui = ui.clone();
            move |_| ui.check_usage()
        });
        // Narrow windows show only the percent.
        if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 480sp") {
            let breakpoint = adw::Breakpoint::new(condition);
            for narrow in [true, false] {
                let ui = ui.clone();
                let redraw = move |_: &adw::Breakpoint| {
                    ui.narrow.set(narrow);
                    ui.redraw_usage();
                };
                if narrow {
                    breakpoint.connect_apply(redraw);
                } else {
                    breakpoint.connect_unapply(redraw);
                }
            }
            ui.window.add_breakpoint(breakpoint);
        }
        // Ages "updated N min ago". It only redraws the line.
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_seconds_local(30, move || match weak.upgrade() {
            Some(ui) => {
                ui.redraw_usage();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        // Enter goes to the composer, which refuses it for a draft not yet
        // reviewed. Either way it never inserts a line (that would count as
        // an edit); Shift+Enter does.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let ui = ui.clone();
            move |_, key, _, modifiers| {
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                    && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                {
                    ui.submit(Via::EnterKey);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        ui.input.add_controller(keys);
        ui.input.buffer().connect_changed({
            let ui = ui.clone();
            move |buffer| {
                if ui.syncing.get() {
                    return;
                }
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                ui.composer.borrow_mut().edited(&text);
                ui.refresh();
            }
        });
        ui.banner.connect_button_clicked({
            let ui = ui.clone();
            move |_| {
                if ui.composer.borrow_mut().discard() {
                    ui.show_input();
                }
                ui.refresh();
            }
        });
        ui.cancel.connect_clicked({
            let ui = ui.clone();
            move |_| ui.cancel_turn()
        });
        ui.dock_toggle.connect_clicked({
            let ui = ui.clone();
            move |_| ui.toggle_mode()
        });
        if let Some(settings) = &ui.mode_settings {
            settings.connect_changed(Some(WINDOW_MODE_KEY), {
                let weak = Rc::downgrade(&ui);
                move |_, _| {
                    if let Some(ui) = weak.upgrade() {
                        ui.apply_look();
                    }
                }
            });
        }
        // The extension owns this name while it is enabled.
        let watcher = gio::bus_watch_name(
            gio::BusType::Session,
            EXTENSION_BUS_NAME,
            gio::BusNameWatcherFlags::NONE,
            {
                let weak = Rc::downgrade(&ui);
                move |_, _, _| {
                    if let Some(ui) = weak.upgrade() {
                        ui.extension_running.set(true);
                        ui.apply_look();
                    }
                }
            },
            {
                let weak = Rc::downgrade(&ui);
                move |_, _| {
                    if let Some(ui) = weak.upgrade() {
                        ui.extension_running.set(false);
                        ui.apply_look();
                    }
                }
            },
        );
        *ui.watcher.borrow_mut() = Some(Box::new(move || gio::bus_unwatch_name(watcher)));
        ui.apply_look();
        // Tag colours (code tint, diff colours) follow the style; the sheet
        // follows by itself. Redrawn once the new style has taken effect.
        let style_manager = adw::StyleManager::default();
        for changed in [
            style_manager.connect_dark_notify({
                let weak = Rc::downgrade(&ui);
                move |_| Ui::restyle_later(&weak)
            }),
            style_manager.connect_high_contrast_notify({
                let weak = Rc::downgrade(&ui);
                move |_| Ui::restyle_later(&weak)
            }),
        ] {
            let manager = style_manager.clone();
            let cell = Cell::new(Some(changed));
            ui.window.connect_destroy(move |_| {
                if let Some(id) = cell.take() {
                    manager.disconnect(id);
                }
            });
        }
        ui.window.connect_close_request({
            let ui = ui.clone();
            move |_| {
                if let Some(unwatch) = ui.watcher.borrow_mut().take() {
                    unwatch();
                }
                ui.close_session();
                glib::Propagation::Proceed
            }
        });
        ui
    }

    fn mode(&self) -> WindowMode {
        self.mode_settings.as_ref().map_or(WindowMode::Docked, |s| {
            WindowMode::parse(&s.string(WINDOW_MODE_KEY))
        })
    }

    /// The header button and the square top corners, from the view-model.
    fn apply_look(&self) {
        let look = window_look(
            self.extension_running.get(),
            self.mode_settings.is_some(),
            self.mode(),
        );
        self.dock_toggle.set_visible(look.toggle_visible);
        self.dock_toggle.set_icon_name(look.icon);
        self.dock_toggle.set_tooltip_text(Some(look.tooltip));
        self.dock_toggle
            .update_property(&[gtk::accessible::Property::Label(look.tooltip)]);
        // Docked: Close only. Floating: the system's own set again.
        if look.close_only {
            let system = gtk::Settings::default()
                .and_then(|settings| settings.gtk_decoration_layout())
                .map(|layout| layout.to_string())
                .unwrap_or_default();
            self.header
                .set_decoration_layout(Some(docked_layout(&system)));
        } else {
            self.header.set_decoration_layout(None);
        }
        if look.docked {
            self.window.add_css_class("vexos-docked");
        } else {
            self.window.remove_css_class("vexos-docked");
        }
    }

    /// Rebuilds every row (its text tags carry theme colours) after the
    /// style changed, once the new style is in place. Does not scroll.
    fn restyle_later(weak: &std::rc::Weak<Ui>) {
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                let count = ui.rows.borrow().len();
                for index in 0..count {
                    ui.replace(index);
                }
            }
        });
    }

    fn toggle_mode(&self) {
        if let Some(settings) = &self.mode_settings {
            let _ = settings.set_string(WINDOW_MODE_KEY, self.mode().toggled().as_str());
        }
    }

    /// Prepares the session; the window is already up. The agent itself
    /// starts in `begin_start`, after the first frame.
    fn start(self: &Rc<Self>, agent: Agent, requested: Instant, built: Instant) {
        if self.conversation.borrow().is_some() {
            return;
        }
        let mut conversation = Conversation::new(agent)
            .usage_on_launch(panel_settings().usage_on_launch)
            .window_requested(requested);
        if let Some(startup) = STARTUP.take() {
            conversation = conversation.startup(startup);
        }
        conversation.built(built);
        *self.conversation.borrow_mut() = Some(conversation);
        self.refresh();

        self.pending_start.set(Some(agent));
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(START_DEADLINE, move || {
            if let Some(ui) = weak.upgrade() {
                ui.begin_start();
            }
        });
    }

    /// Starts the session in the background, once: after the first frame, or
    /// at the deadline. Closing the window first cancels it.
    fn begin_start(self: &Rc<Self>) {
        let Some(agent) = self.pending_start.take() else {
            return;
        };
        self.with_conversation(|c| {
            c.spawn_requested(Instant::now());
            Vec::new()
        });
        let starting = runtime().spawn(Session::spawn(agent, Config::new(WORKDIR)));
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let mut events = match starting.await {
                Ok(Ok((session, events))) => {
                    *ui.session.borrow_mut() = Some(Arc::new(session));
                    ui.with_conversation(|c| {
                        c.started();
                        Vec::new()
                    });
                    events
                }
                Ok(Err(e)) => return ui.with_conversation(|c| c.failed(&e.to_string())),
                Err(e) => return ui.with_conversation(|c| c.failed(&e.to_string())),
            };
            // `Events` is a plain channel; it needs no tokio reactor.
            while let Some(event) = events.recv().await {
                ui.with_conversation(|c| c.apply(event));
            }
        });
    }

    fn session(&self) -> Option<Arc<Session>> {
        self.session.borrow().clone()
    }

    /// Runs `f` on the conversation, then redraws the items it reports. If
    /// `f` made the session ready, the view-model hands out the launch
    /// `/usage` (once per conversation, only right then), and it is sent.
    fn with_conversation(self: &Rc<Self>, f: impl FnOnce(&mut Conversation) -> Vec<usize>) {
        let (changed, launch) = match self.conversation.borrow_mut().as_mut() {
            Some(c) => {
                let changed = f(c);
                (changed, c.take_launch_usage())
            }
            None => return,
        };
        for index in changed {
            self.render(index);
        }
        self.refresh();
        if let Some(out) = launch
            && self.session().is_some()
        {
            self.send_prompt(out);
        }
    }

    /// Fills the input from the composer.
    fn show_input(&self) {
        let text = self.composer.borrow().text().to_owned();
        self.syncing.set(true);
        self.input.buffer().set_text(&text);
        self.syncing.set(false);
    }

    /// A notification's draft: into the input when the conversation can
    /// take it, held until then. Never sent from here.
    fn hold_draft(self: &Rc<Self>, draft: String) {
        self.composer.borrow_mut().hold(draft);
        self.refresh();
    }

    /// The input has text and a draft arrived: replace, add below, or keep.
    fn ask_placement(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::new(
            Some("Use the draft from the notification?"),
            Some(
                "The input already has text. Replace it with the draft, add the draft below it, \
                 or keep your text and drop the draft. Nothing is sent either way.",
            ),
        );
        dialog.add_responses(&[
            ("cancel", "_Keep My Text"),
            ("append", "_Add Below"),
            ("replace", "_Replace"),
        ]);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let placement = match dialog.choose_future(Some(&ui.window)).await.as_str() {
                "replace" => Placement::Replace,
                "append" => Placement::Append,
                _ => Placement::Cancel,
            };
            if ui.composer.borrow_mut().resolve(placement) {
                ui.show_input();
            }
            ui.refresh();
        });
    }

    /// The Send button or Enter: the composer decides whether this sends.
    fn submit(self: &Rc<Self>, via: Via) {
        let can_send = self
            .conversation
            .borrow()
            .as_ref()
            .is_some_and(|c| c.can_send());
        let out = self.composer.borrow_mut().submit(via, can_send);
        if let Some(out) = out {
            self.send_prompt(out);
        }
    }

    /// A click on the usage line: the view-model decides, and its message is
    /// the constant `/usage`, shown with its full reply. The input is left as
    /// it is: with a session and a message, `send_prompt` sends rather than
    /// restoring, and a refused `/usage` is never handed back to the input
    /// (`turn_ended`).
    fn check_usage(self: &Rc<Self>) {
        if self.session().is_none() {
            return;
        }
        let out = self
            .conversation
            .borrow()
            .as_ref()
            .and_then(Conversation::check_usage);
        if let Some(out) = out {
            self.send_prompt(out);
        }
    }

    /// The one place a prompt goes to the agent, with an `Outgoing` from the
    /// composer or the view-model's `/usage`.
    fn send_prompt(self: &Rc<Self>, out: Outgoing) {
        let sent = match self.session() {
            Some(_) => self
                .conversation
                .borrow_mut()
                .as_mut()
                .and_then(|c| c.send(&out)),
            None => None,
        };
        let (Some(session), Some(sent)) = (self.session(), sent) else {
            self.composer.borrow_mut().restore(out.into_text());
            self.show_input();
            self.refresh();
            return;
        };
        let text = out.into_text();
        self.show_input();
        match sent {
            Sent::Shown(index) => self.render(index),
            // The launch `/usage` holds the input for at most `USAGE_LOCK`;
            // then the input opens and the turn keeps running.
            Sent::Hidden => {
                let ui = self.clone();
                glib::timeout_add_local_once(USAGE_LOCK, move || {
                    if let Some(c) = ui.conversation.borrow_mut().as_mut() {
                        c.release_usage_lock();
                    }
                    ui.refresh();
                });
            }
        }
        self.refresh();
        let turn = runtime().spawn(async move { session.prompt(&text).await });
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let result = turn
                .await
                .unwrap_or_else(|e| Err(Error::Protocol(e.to_string())));
            ui.with_conversation(|c| match sent {
                Sent::Hidden => c.usage_ended(result),
                Sent::Shown(_) => {
                    let changed = c.turn_ended(result);
                    // A refused message goes back in the input, before a
                    // waiting draft is offered.
                    if let Some(text) = c.take_unsent() {
                        ui.composer.borrow_mut().restore(text);
                        ui.show_input();
                    }
                    changed
                }
            });
        });
    }

    fn cancel_turn(self: &Rc<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        self.with_conversation(|c| c.cancelled());
        runtime().spawn(async move { session.cancel().await });
    }

    fn click(self: &Rc<Self>, request: u64, decision: Decision) {
        let Some(session) = self.session() else {
            return;
        };
        self.with_conversation(|c| {
            let _ = c.click(request, decision, &*session);
            // Redraw the card whatever happened: answered, or withdrawn.
            c.card_rows(request)
        });
    }

    /// Denies what is pending, then stops the agent.
    fn close_session(self: &Rc<Self>) {
        self.pending_start.set(None);
        let Some(session) = self.session.borrow_mut().take() else {
            return;
        };
        self.with_conversation(|c| c.close(&*session));
        // The session's drop hands the rest to the runtime.
        let _guard = runtime().enter();
        drop(session);
    }

    fn refresh(self: &Rc<Self>) {
        let conversation = self.conversation.borrow();
        let Some(c) = conversation.as_ref() else {
            return;
        };
        // Focus is not moved to the input.
        let offer = self.composer.borrow_mut().offer(c.can_send());
        match offer {
            Offer::Placed => self.show_input(),
            Offer::Ask => self.ask_placement(),
            Offer::Nothing => {}
        }
        self.banner.set_revealed(self.composer.borrow().banner());
        let state = match c.state() {
            State::Starting => "starting…".to_owned(),
            State::Ended(_) => "ended".to_owned(),
            State::Ready | State::Busy => match c.mode() {
                Some(mode) => mode_label(mode),
                None => "mode not confirmed yet".to_owned(),
            },
        };
        self.title
            .set_subtitle(&format!("{} · {state}", agent_label(c.agent())));
        self.input.set_sensitive(c.can_send());
        self.send.set_sensitive(c.can_send());
        self.lock_reveal.set_reveal_child(c.usage_locked());
        let note = c.starting_note(self.composer.borrow().waiting());
        self.starting_note.set_label(note.as_deref().unwrap_or(""));
        self.starting_note.set_visible(note.is_some());
        let empty = c.empty_state();
        self.stack
            .set_visible_child_name(if empty.is_some() { "empty" } else { "list" });
        self.empty_spinner
            .set_visible(matches!(empty, Some(Empty::Starting(_))));
        if let Some(Empty::Ready(agent)) = empty {
            self.empty_title.set_label(&format!("{agent} is ready"));
        }
        let ready = matches!(empty, Some(Empty::Ready(_)));
        self.empty_title.set_visible(ready);
        self.empty_hint.set_visible(ready);
        self.title.set_tooltip_text(c.launch_summary().as_deref());
        self.cancel.set_sensitive(c.turn_running());
        self.show_usage(c);
    }

    /// The usage line from the view-model: its label, accessible name,
    /// tooltip and state. Nothing else sets its text.
    fn show_usage(&self, c: &Conversation) {
        let line = c.usage_button(Instant::now());
        let text = if self.narrow.get() {
            line.narrow
        } else {
            line.wide
        };
        self.usage.set_label(&text);
        self.usage
            .update_property(&[gtk::accessible::Property::Label(&text)]);
        self.usage.set_tooltip_text(Some(line.tooltip));
        self.usage.set_visible(line.visible);
        self.usage.set_sensitive(line.enabled);
    }

    /// Redraws only the usage line: for its age and the window width.
    fn redraw_usage(&self) {
        if let Some(c) = self.conversation.borrow().as_ref() {
            self.show_usage(c);
        }
    }

    fn render(self: &Rc<Self>, index: usize) {
        self.replace(index);
        let adjustment = self.scroller.vadjustment();
        glib::idle_add_local_once(move || adjustment.set_value(adjustment.upper()));
    }

    /// Draws row `index` from the conversation, in place or at the end.
    fn replace(self: &Rc<Self>, index: usize) {
        let widget = match self.conversation.borrow().as_ref() {
            Some(c) => self.widget(c, &c.items()[index]),
            None => return,
        };
        let mut rows = self.rows.borrow_mut();
        if let Some(old) = rows.get(index) {
            self.list.insert_child_after(&widget, Some(old));
            self.list.remove(old);
            rows[index] = widget;
        } else {
            self.list.append(&widget);
            rows.push(widget);
        }
    }

    fn widget(self: &Rc<Self>, c: &Conversation, item: &Item) -> gtk::Widget {
        match item {
            Item::User(text) => {
                let label = text_label(text);
                label.set_halign(gtk::Align::End);
                label.set_max_width_chars(60);
                label.add_css_class("vexos-user");
                label.upcast()
            }
            Item::Agent(text) => reply(text, &TagColors::of(&self.window)).upcast(),
            Item::Tool(card) => {
                let name = card
                    .title
                    .as_deref()
                    .or(card.kind.as_deref())
                    .unwrap_or(&card.id);
                let status = c.tool_status(card);
                let label = text_label(&format!("{name} — {status}"));
                label.add_css_class("vexos-quiet");
                label.add_css_class("caption");
                label.upcast()
            }
            Item::Notice(text) => {
                let label = text_label(text);
                label.add_css_class("vexos-quiet");
                label.upcast()
            }
            Item::Approval(card) => self.approval(card, &TagColors::of(&self.window)).upcast(),
        }
    }

    fn approval(self: &Rc<Self>, card: &ApprovalCard, colors: &TagColors) -> gtk::Box {
        let frame = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        frame.add_css_class("card");
        frame.add_css_class("vexos-approval");
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(16)
            .margin_end(16)
            .build();
        frame.append(&inner);
        let heading = text_label(&format!("Approve: {}", card.title));
        heading.add_css_class("heading");
        inner.append(&heading);
        inner.append(&proposal(&card.proposal, colors));

        let buttons = card.buttons();
        if buttons.is_empty() {
            let answer = match card.answer {
                Answer::Pending => "Nothing to approve: the agent offered no one-time choice.",
                Answer::Approved => "Approved once.",
                Answer::Denied => "Denied.",
                Answer::Withdrawn => "No longer pending. Not approved.",
                Answer::Closed => "Denied: the session closed.",
                Answer::TimedOut => "Timed out, denied.",
            };
            let label = text_label(answer);
            label.add_css_class("vexos-quiet");
            inner.append(&label);
        } else {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_halign(gtk::Align::End);
            for (decision, text) in buttons {
                let button = gtk::Button::with_label(text);
                button.add_css_class("vexos-choice");
                // Denying is the safe answer: a plain button, not a red one.
                if decision == Decision::ApproveOnce {
                    button.add_css_class("suggested-action");
                }
                let (ui, request) = (self.clone(), card.request);
                button.connect_clicked(move |_| ui.click(request, decision));
                row.append(&button);
            }
            inner.append(&row);
        }
        frame
    }
}

fn text_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .build()
}

/// An agent reply, formatted. Each run is inserted as literal text with a tag
/// per style; nothing here is parsed as markup.
fn reply(text: &str, colors: &TagColors) -> gtk::TextView {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(0)
        .right_margin(0)
        .build();
    view.add_css_class("vexos-reply");
    let buffer = view.buffer();
    buffer.create_tag(Some("bold"), &[("weight", &700)]);
    buffer.create_tag(Some("italic"), &[("style", &gtk::pango::Style::Italic)]);
    buffer.create_tag(
        Some("code"),
        &[
            ("family", &"monospace"),
            ("background-rgba", &colors.code_background),
        ],
    );
    buffer.create_tag(Some("heading"), &[("scale", &1.15), ("weight", &700)]);
    for run in markdown(text) {
        let style = run.style;
        let tags: Vec<&str> = [
            (style.bold, "bold"),
            (style.italic, "italic"),
            (style.code, "code"),
            (style.heading, "heading"),
        ]
        .into_iter()
        .filter_map(|(on, tag)| on.then_some(tag))
        .collect();
        let mut end = buffer.end_iter();
        buffer.insert_with_tags_by_name(&mut end, &run.text, &tags);
    }
    view
}

fn monospace(text: &str) -> gtk::Label {
    let label = text_label(text);
    label.add_css_class("monospace");
    label
}

/// The tinted box around a command or a change.
fn code_box(child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("vexos-code");
    frame.append(child);
    frame.upcast()
}

/// The change or command a card asks about.
fn proposal(proposal: &Proposal, colors: &TagColors) -> gtk::Widget {
    match proposal {
        Proposal::Command(command) => code_box(&monospace(&format!("$ {command}"))),
        Proposal::Plan(plan) if plan.trim().is_empty() => {
            text_label("The agent sent no plan with this request.").upcast()
        }
        Proposal::Plan(plan) => text_label(plan).upcast(),
        Proposal::Other(json) => code_box(&monospace(json)),
        Proposal::Diff { path, lines } => {
            let view = gtk::TextView::builder()
                .editable(false)
                .cursor_visible(false)
                .monospace(true)
                .wrap_mode(gtk::WrapMode::WordChar)
                .build();
            let buffer = view.buffer();
            for (name, color) in [
                ("added", &colors.added),
                ("removed", &colors.removed),
                ("hunk", &colors.hunk),
            ] {
                buffer.create_tag(Some(name), &[("foreground-rgba", color)]);
            }
            for line in lines {
                let (tag, text) = match line {
                    DiffLine::Added(l) => (Some("added"), format!("+ {l}\n")),
                    DiffLine::Removed(l) => (Some("removed"), format!("- {l}\n")),
                    DiffLine::Hunk(l) => (Some("hunk"), format!("{l}\n")),
                    DiffLine::Context(l) => (None, format!("  {l}\n")),
                };
                let mut end = buffer.end_iter();
                match tag {
                    Some(tag) => buffer.insert_with_tags_by_name(&mut end, &text, &[tag]),
                    None => buffer.insert(&mut end, &text),
                }
            }
            let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
            column.append(&monospace(path));
            column.append(&view);
            code_box(&column)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_opens_shows_or_hides_and_never_opens_a_second_window() {
        assert_eq!(toggle(None), Toggle::Open);
        assert_eq!(toggle(Some(false)), Toggle::Show);
        assert_eq!(toggle(Some(true)), Toggle::Hide);
    }
}
