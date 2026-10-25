//! The VexOS assistant panel: a plain window over `vexos-ai-viewmodel`. It
//! renders what the view-model holds and forwards clicks to it; the rules
//! (what reaches the user, what counts as an answer) live in the core and the
//! view-model, which are tested headless.

mod rebuild;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use adw::prelude::*;
use gtk::glib;
use vexos_ai_core::{Agent, Config, Decision, Error, Session};
use vexos_ai_viewmodel::{
    Answer, ApprovalCard, Conversation, DiffLine, Item, Proposal, Rebuild, State, agent_label,
    mode_label,
};

const APP_ID: &str = "io.github.victorytek.VexosAssistant";
/// Where every session runs, as `vexos-ai` does in the terminal.
const WORKDIR: &str = "/etc/nixos";

/// The core is async on tokio; GTK owns the main thread.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("start the tokio runtime"))
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| Ui::build(app).window.present());
    app.run()
}

struct Ui {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    list: gtk::Box,
    scroller: gtk::ScrolledWindow,
    rows: RefCell<Vec<gtk::Widget>>,
    entry: gtk::Entry,
    send: gtk::Button,
    cancel: gtk::Button,
    rebuild_button: gtk::Button,
    conversation: RefCell<Option<Conversation>>,
    session: RefCell<Option<Arc<Session>>>,
    rebuild: RefCell<Rebuild>,
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
        let start = adw::StatusPage::builder()
            .title("Start a session")
            .description("Choose the agent for this session. It works in /etc/nixos and asks before it changes anything.")
            .child(&choices)
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
        let entry = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text("Ask about this system…")
            .sensitive(false)
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
        composer.append(&entry);
        composer.append(&send);
        let chat = gtk::Box::new(gtk::Orientation::Vertical, 0);
        chat.append(&scroller);
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
            entry,
            send,
            cancel,
            rebuild_button,
            conversation: RefCell::new(None),
            session: RefCell::new(None),
            rebuild: RefCell::new(Rebuild::default()),
        });

        for (button, agent) in [(&claude, Agent::Claude), (&opencode, Agent::OpenCode)] {
            let ui = ui.clone();
            button.connect_clicked(move |_| ui.start(agent));
        }
        let on_send = {
            let ui = ui.clone();
            move || ui.send_prompt()
        };
        ui.send.connect_clicked({
            let on_send = on_send.clone();
            move |_| on_send()
        });
        ui.entry.connect_activate(move |_| on_send());
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

    fn send_prompt(self: &Rc<Self>) {
        let text = self.entry.text().to_string();
        let (Some(session), Some(index)) = (
            self.session(),
            self.conversation
                .borrow_mut()
                .as_mut()
                .and_then(|c| c.send(&text)),
        ) else {
            return;
        };
        self.entry.set_text("");
        self.render(index);
        self.refresh();
        let turn = runtime().spawn(async move { session.prompt(&text).await });
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let result = turn
                .await
                .unwrap_or_else(|e| Err(Error::Protocol(e.to_string())));
            ui.with_conversation(|c| c.turn_ended(result));
            // A refused message goes back in the input.
            if let Some(text) = ui
                .conversation
                .borrow_mut()
                .as_mut()
                .and_then(|c| c.take_unsent())
            {
                ui.entry.set_text(&text);
            }
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
            c.items()
                .iter()
                .position(|item| matches!(item, Item::Approval(card) if card.request == request))
                .into_iter()
                .collect()
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

    fn refresh(&self) {
        self.rebuild_button.set_sensitive(matches!(
            *self.rebuild.borrow(),
            Rebuild::Idle | Rebuild::Finished(_)
        ));
        let conversation = self.conversation.borrow();
        let Some(c) = conversation.as_ref() else {
            return;
        };
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
        self.entry.set_sensitive(c.can_send());
        self.send.set_sensitive(c.can_send());
        self.cancel.set_sensitive(*c.state() == State::Busy);
    }

    fn render(self: &Rc<Self>, index: usize) {
        let widget = match self.conversation.borrow().as_ref() {
            Some(c) => self.widget(&c.items()[index]),
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

    fn widget(self: &Rc<Self>, item: &Item) -> gtk::Widget {
        match item {
            Item::User(text) => {
                let label = text_label(text);
                label.set_xalign(1.0);
                label.add_css_class("heading");
                label.upcast()
            }
            Item::Agent(text) => text_label(text).upcast(),
            Item::Tool(card) => {
                let name = card
                    .title
                    .as_deref()
                    .or(card.kind.as_deref())
                    .unwrap_or(&card.id);
                let status = card.status.as_deref().unwrap_or("pending");
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
