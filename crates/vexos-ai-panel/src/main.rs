//! The VexOS assistant panel: a plain window over `vexos-ai-viewmodel`. It
//! renders what the view-model holds and forwards clicks to it; the rules
//! (what reaches the user, what counts as an answer) live in the core and the
//! view-model, which are tested headless.

mod notify;
mod rebuild;
#[cfg(test)]
mod scan_tests;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use vexos_ai_core::{Agent, Config, Decision, Error, Session};
use vexos_ai_viewmodel::{
    Answer, ApprovalCard, Composer, Conversation, DiffLine, Item, Offer, Outgoing, Placement,
    Proposal, Rebuild, State, Via, agent_label, markdown, mode_label,
};

const APP_ID: &str = "io.github.victorytek.VexosAssistant";
/// Where every session runs, as `vexos-ai` does in the terminal.
const WORKDIR: &str = "/etc/nixos";

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
    let app = adw::Application::builder().application_id(APP_ID).build();
    let slot = Slot::default();
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
    let ui = Ui::build(app);
    // Runs after the session's own close handler.
    ui.window.connect_close_request({
        let slot = slot.clone();
        move |_| {
            slot.borrow_mut().take();
            glib::Propagation::Proceed
        }
    });
    *slot.borrow_mut() = Some(ui.clone());
    ui
}

struct Ui {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    list: gtk::Box,
    scroller: gtk::ScrolledWindow,
    rows: RefCell<Vec<gtk::Widget>>,
    input: gtk::TextView,
    send: gtk::Button,
    cancel: gtk::Button,
    rebuild_button: gtk::Button,
    conversation: RefCell<Option<Conversation>>,
    session: RefCell<Option<Arc<Session>>>,
    rebuild: RefCell<Rebuild>,
    rebuild_view: RefCell<Option<rebuild::RebuildView>>,
    /// What is in the input, and whether it is an unsent draft. The only
    /// source of a message to send.
    composer: RefCell<Composer>,
    /// Set while the input is filled from `composer`, so the buffer's own
    /// change signals are not taken for the user's edits.
    syncing: Cell<bool>,
    /// On the agent-choice page: a draft is waiting.
    waiting: gtk::Label,
    /// Above the input while it holds a draft: review, Send, or Discard.
    banner: adw::Banner,
}

impl Ui {
    fn build(app: &adw::Application) -> Rc<Ui> {
        let title = adw::WindowTitle::new("VexOS Assistant", "Choose an agent");
        let cancel = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text("Cancel the current turn")
            .sensitive(false)
            .build();
        let rebuild_button = gtk::Button::with_label("Run just rebuild");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&cancel);
        header.pack_end(&rebuild_button);

        // The agent is chosen once, here; there is no way back to this page.
        let claude = gtk::Button::with_label(agent_label(Agent::Claude));
        let opencode = gtk::Button::with_label(agent_label(Agent::OpenCode));
        let choices = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        choices.set_halign(gtk::Align::Center);
        for button in [&claude, &opencode] {
            button.add_css_class("pill");
            button.add_css_class("suggested-action");
            choices.append(button);
        }
        let waiting = gtk::Label::builder()
            .label("A draft from a notification is waiting. It goes into the input once the session is ready; nothing is sent until you press Send.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .visible(false)
            .build();
        waiting.add_css_class("dim-label");
        let start_body = gtk::Box::new(gtk::Orientation::Vertical, 18);
        start_body.append(&choices);
        start_body.append(&waiting);
        let start = adw::StatusPage::builder()
            .title("Start a session")
            .description("Choose the agent for this session. It works in /etc/nixos and asks before it changes anything.")
            .child(&start_body)
            .build();

        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
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
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(6)
            .right_margin(6)
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
        let composer = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        composer.append(&input_scroller);
        composer.append(&send);
        let chat = gtk::Box::new(gtk::Orientation::Vertical, 0);
        chat.append(&scroller);
        chat.append(&banner);
        chat.append(&composer);

        let stack = gtk::Stack::new();
        stack.add_named(&start, Some("start"));
        stack.add_named(&chat, Some("chat"));
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.set_content(Some(&stack));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("VexOS Assistant")
            .default_width(760)
            .default_height(820)
            .content(&content)
            .build();

        let ui = Rc::new(Ui {
            window,
            title,
            stack,
            list,
            scroller,
            rows: RefCell::new(Vec::new()),
            input,
            send,
            cancel,
            rebuild_button,
            conversation: RefCell::new(None),
            session: RefCell::new(None),
            rebuild: RefCell::new(Rebuild::default()),
            rebuild_view: RefCell::new(None),
            composer: RefCell::new(Composer::default()),
            syncing: Cell::new(false),
            waiting,
            banner,
        });

        for (button, agent) in [(&claude, Agent::Claude), (&opencode, Agent::OpenCode)] {
            let ui = ui.clone();
            button.connect_clicked(move |_| ui.start(agent));
        }
        ui.send.connect_clicked({
            let ui = ui.clone();
            move |_| ui.submit(Via::SendButton)
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
        ui.rebuild_button.connect_clicked({
            let ui = ui.clone();
            move |_| rebuild::ask(&ui)
        });
        ui.window.connect_close_request({
            let ui = ui.clone();
            move |_| {
                ui.close_session();
                glib::Propagation::Proceed
            }
        });
        ui
    }

    fn start(self: &Rc<Self>, agent: Agent) {
        if self.conversation.borrow().is_some() {
            return;
        }
        *self.conversation.borrow_mut() = Some(Conversation::new(agent));
        self.stack.set_visible_child_name("chat");
        self.refresh();

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

    /// Runs `f` on the conversation, then redraws the items it reports.
    fn with_conversation(self: &Rc<Self>, f: impl FnOnce(&mut Conversation) -> Vec<usize>) {
        let changed = match self.conversation.borrow_mut().as_mut() {
            Some(c) => f(c),
            None => return,
        };
        for index in changed {
            self.render(index);
        }
        self.refresh();
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

    /// The one place a prompt goes to the agent, with what the composer let
    /// out.
    fn send_prompt(self: &Rc<Self>, out: Outgoing) {
        let index = match self.session() {
            Some(_) => self
                .conversation
                .borrow_mut()
                .as_mut()
                .and_then(|c| c.send(&out)),
            None => None,
        };
        let (Some(session), Some(index)) = (self.session(), index) else {
            self.composer.borrow_mut().restore(out.into_text());
            self.show_input();
            self.refresh();
            return;
        };
        let text = out.into_text();
        self.show_input();
        self.render(index);
        self.refresh();
        let turn = runtime().spawn(async move { session.prompt(&text).await });
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let result = turn
                .await
                .unwrap_or_else(|e| Err(Error::Protocol(e.to_string())));
            ui.with_conversation(|c| {
                let changed = c.turn_ended(result);
                // A refused message goes back in the input, before a waiting
                // draft is offered.
                if let Some(text) = c.take_unsent() {
                    ui.composer.borrow_mut().restore(text);
                    ui.show_input();
                }
                changed
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
        let Some(session) = self.session.borrow_mut().take() else {
            return;
        };
        self.with_conversation(|c| c.close(&*session));
        // The session's drop hands the rest to the runtime.
        let _guard = runtime().enter();
        drop(session);
    }

    fn refresh(self: &Rc<Self>) {
        self.rebuild_button.set_sensitive(matches!(
            *self.rebuild.borrow(),
            Rebuild::Idle | Rebuild::Finished(_)
        ));
        let conversation = self.conversation.borrow();
        self.waiting
            .set_visible(conversation.is_none() && self.composer.borrow().waiting());
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
        self.cancel.set_sensitive(*c.state() == State::Busy);
    }

    fn render(self: &Rc<Self>, index: usize) {
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
        let adjustment = self.scroller.vadjustment();
        glib::idle_add_local_once(move || adjustment.set_value(adjustment.upper()));
    }

    fn widget(self: &Rc<Self>, c: &Conversation, item: &Item) -> gtk::Widget {
        match item {
            Item::User(text) => {
                let label = text_label(text);
                label.set_xalign(1.0);
                label.add_css_class("heading");
                label.upcast()
            }
            Item::Agent(text) => reply(text).upcast(),
            Item::Tool(card) => {
                let name = card
                    .title
                    .as_deref()
                    .or(card.kind.as_deref())
                    .unwrap_or(&card.id);
                let status = c.tool_status(card);
                let label = text_label(&format!("{name} — {status}"));
                label.add_css_class("dim-label");
                label.add_css_class("caption");
                label.upcast()
            }
            Item::Notice(text) => {
                let label = text_label(text);
                label.add_css_class("dim-label");
                label.upcast()
            }
            Item::Approval(card) => self.approval(card).upcast(),
        }
    }

    fn approval(self: &Rc<Self>, card: &ApprovalCard) -> gtk::Box {
        let frame = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        frame.add_css_class("card");
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        frame.append(&inner);
        let heading = text_label(&format!("Approve: {}", card.title));
        heading.add_css_class("heading");
        inner.append(&heading);
        inner.append(&proposal(&card.proposal));

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
            label.add_css_class("dim-label");
            inner.append(&label);
        } else {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_halign(gtk::Align::End);
            for (decision, text) in buttons {
                let button = gtk::Button::with_label(text);
                button.add_css_class(match decision {
                    Decision::ApproveOnce => "suggested-action",
                    Decision::DenyOnce => "destructive-action",
                });
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
fn reply(text: &str) -> gtk::TextView {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let buffer = view.buffer();
    buffer.create_tag(Some("bold"), &[("weight", &700)]);
    buffer.create_tag(Some("italic"), &[("style", &gtk::pango::Style::Italic)]);
    buffer.create_tag(Some("code"), &[("family", &"monospace")]);
    buffer.create_tag(Some("heading"), &[("scale", &1.2)]);
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

/// The change or command a card asks about.
fn proposal(proposal: &Proposal) -> gtk::Widget {
    match proposal {
        Proposal::Command(command) => monospace(&format!("$ {command}")).upcast(),
        Proposal::Plan(plan) if plan.trim().is_empty() => {
            text_label("The agent sent no plan with this request.").upcast()
        }
        Proposal::Plan(plan) => text_label(plan).upcast(),
        Proposal::Other(json) => monospace(json).upcast(),
        Proposal::Diff { path, lines } => {
            let view = gtk::TextView::builder()
                .editable(false)
                .cursor_visible(false)
                .monospace(true)
                .wrap_mode(gtk::WrapMode::WordChar)
                .build();
            let buffer = view.buffer();
            for (name, color) in [
                ("added", "#26a269"),
                ("removed", "#e01b24"),
                ("hunk", "#77767b"),
            ] {
                buffer.create_tag(Some(name), &[("foreground", &color)]);
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
            column.upcast()
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
