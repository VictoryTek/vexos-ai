//! The session behind the front door: the agent's session, the conversation and
//! the composer, driven by calls that already passed the gate. It does what the
//! GTK panel's window did, without a window: the view-model decides, this only
//! forwards.

use std::cell::{Cell, Ref, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use tokio::runtime::Handle;
use vexos_ai_core::{Agent, Config as CoreConfig, Decision, Error, Events, Session};
use vexos_ai_viewmodel::{
    Answer, Composer, Conversation, Item, Outgoing, Placement as Place, Sent, USAGE_LOCK,
    Via as How,
};

use crate::approval::Approvals;
use crate::bus::{Handler, HandlerError, Reply};
use crate::gate::{Call, Choice, Placement, Via};

/// Where the agent works: the same fixed place as the panel. Nothing a caller
/// sends can change it.
pub const WORKDIR: &str = "/etc/nixos";

/// What a session start returns.
pub type Started = Result<(Session, Events), Error>;

/// Starts the agent's session. The real one runs the agent; a test connects a
/// fake one.
pub trait Starter {
    fn start(&self, agent: Agent) -> Pin<Box<dyn Future<Output = Started>>>;
}

/// The program's own settings: read once, by the helper, from the user's files.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub agent: Agent,
    pub usage_on_launch: bool,
}

/// Runs the saved agent, as the panel does.
pub struct Agents {
    runtime: Handle,
}

impl Agents {
    pub fn new(runtime: Handle) -> Agents {
        Agents { runtime }
    }
}

impl Starter for Agents {
    fn start(&self, agent: Agent) -> Pin<Box<dyn Future<Output = Started>>> {
        let starting = self
            .runtime
            .spawn(Session::spawn(agent, CoreConfig::new(WORKDIR)));
        Box::pin(async move {
            starting
                .await
                .unwrap_or_else(|e| Err(Error::Protocol(e.to_string())))
        })
    }
}

struct Inner {
    runtime: Handle,
    starter: Box<dyn Starter>,
    settings: Settings,
    conversation: RefCell<Option<Conversation>>,
    composer: RefCell<Composer>,
    session: RefCell<Option<Arc<Session>>>,
    /// Which cards were shown, and when; the session's generation number.
    approvals: RefCell<Approvals>,
    /// The time `Shown` and `Decide` are measured in.
    clock: Rc<dyn Fn() -> Instant>,
    /// `Close` or the Shell's exit came: a session that is still starting is
    /// dropped when it arrives.
    closed: Cell<bool>,
}

pub struct Service {
    inner: Rc<Inner>,
}

impl Service {
    /// `generation` names this session: every `Shown` and `Decide` must carry it.
    pub fn new(
        runtime: Handle,
        starter: Box<dyn Starter>,
        settings: Settings,
        generation: u64,
        clock: Rc<dyn Fn() -> Instant>,
    ) -> Service {
        Service {
            inner: Rc::new(Inner {
                runtime,
                starter,
                settings,
                conversation: RefCell::new(None),
                composer: RefCell::new(Composer::default()),
                session: RefCell::new(None),
                approvals: RefCell::new(Approvals::new(generation)),
                clock,
                closed: Cell::new(false),
            }),
        }
    }

    /// What the conversation holds (read only).
    pub fn conversation(&self) -> Ref<'_, Option<Conversation>> {
        self.inner.conversation.borrow()
    }

    /// What the composer holds (read only).
    pub fn composer(&self) -> Ref<'_, Composer> {
        self.inner.composer.borrow()
    }

    /// Ends the session, denying what is pending. Safe to call again.
    pub fn end(&self) {
        self.inner.close_session();
    }
}

impl Handler for Service {
    fn handle(&self, call: Call) -> Result<Reply, HandlerError> {
        let inner = &self.inner;
        // After `Close` (or the Shell going away) the helper is on its way out:
        // a call that arrives in that gap must not look as if it did something.
        if inner.closed.get() && call != Call::Close {
            return Err(HandlerError::NotReady);
        }
        match call {
            Call::Open => {
                inner.open();
                return Ok(Reply::Generation(inner.approvals.borrow().generation()));
            }
            Call::Close => {
                inner.close_session();
                return Ok(Reply::DoneThenExit);
            }
            Call::Edited(text) => inner.composer.borrow_mut().edited(&text),
            Call::EditLost => inner.composer.borrow_mut().edit_lost(),
            Call::Send { text, via } => inner.submit(&text, via),
            Call::CheckUsage => inner.check_usage(),
            Call::Cancel => inner.cancel_turn(),
            Call::DraftChoice(placement) => {
                let placement = match placement {
                    Placement::Replace => Place::Replace,
                    Placement::Append => Place::Append,
                    Placement::Keep => Place::Cancel,
                };
                inner.composer.borrow_mut().resolve(placement);
            }
            Call::DraftDiscard => {
                inner.composer.borrow_mut().discard();
            }
            Call::Shown {
                generation,
                request,
            } => return inner.shown(generation, request),
            Call::Decide {
                generation,
                request,
                choice,
            } => return inner.decide(generation, request, choice),
        }
        Ok(Reply::Done)
    }

    fn rate_exempt(&self, call: &Call) -> bool {
        let Call::Shown {
            generation,
            request,
        } = call
        else {
            return false;
        };
        let pending = self.inner.is_pending(*request);
        self.inner
            .approvals
            .borrow()
            .first_shown(*generation, *request, pending)
    }

    fn shell_gone(&self) -> bool {
        if self.inner.conversation.borrow().is_none() {
            return false;
        }
        self.inner.close_session();
        true
    }
}

impl Inner {
    fn session(&self) -> Option<Arc<Session>> {
        self.session.borrow().clone()
    }

    /// Whether `request` is a card that is waiting for an answer right now.
    /// A card too large to review, an answered one and an unknown id are not.
    fn is_pending(&self, request: u64) -> bool {
        self.conversation.borrow().as_ref().is_some_and(|c| {
            c.items().iter().any(|item| {
                matches!(item, Item::Approval(card)
                    if card.request == request && card.answer == Answer::Pending)
            })
        })
    }

    /// `Shown`: the Shell drew the card. Starts the clock for `approve-once`.
    fn shown(&self, generation: u64, request: u64) -> Result<Reply, HandlerError> {
        let pending = self.is_pending(request);
        let now = (self.clock)();
        match self
            .approvals
            .borrow_mut()
            .shown(generation, request, pending, now)
        {
            Ok(()) => Ok(Reply::Done),
            Err(why) => {
                eprintln!("vexos-ai-helper: refused Shown: {why:?}");
                Err(HandlerError::Refused)
            }
        }
    }

    /// `Decide`: the only way a request is answered by a caller. The rules
    /// decide first; only then does the view-model's own click pass the answer
    /// to the core.
    fn decide(&self, generation: u64, request: u64, choice: Choice) -> Result<Reply, HandlerError> {
        let pending = self.is_pending(request);
        let now = (self.clock)();
        if let Err(why) = self
            .approvals
            .borrow()
            .decide(generation, request, choice, pending, now)
        {
            eprintln!("vexos-ai-helper: refused Decide: {why:?}");
            return Err(HandlerError::Refused);
        }
        let Some(session) = self.session() else {
            return Err(HandlerError::Refused);
        };
        let decision = match choice {
            Choice::ApproveOnce => Decision::ApproveOnce,
            Choice::DenyOnce => Decision::DenyOnce,
        };
        let clicked = match self.conversation.borrow_mut().as_mut() {
            Some(c) => c.click(request, decision, &*session),
            None => return Err(HandlerError::Refused),
        };
        match clicked {
            Ok(_) => Ok(Reply::Done),
            Err(why) => {
                eprintln!("vexos-ai-helper: refused Decide: {why:?}");
                Err(HandlerError::Refused)
            }
        }
    }

    /// Prepares the conversation and starts the saved agent's session in the
    /// background. A second `Open` does nothing.
    fn open(self: &Rc<Self>) {
        if self.conversation.borrow().is_some() {
            return;
        }
        let agent = self.settings.agent;
        *self.conversation.borrow_mut() =
            Some(Conversation::new(agent).usage_on_launch(self.settings.usage_on_launch));
        let starting = self.starter.start(agent);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let mut events = match starting.await {
                Ok((session, events)) => {
                    if ui.closed.get() {
                        // Closed while it started: it is never used.
                        let _guard = ui.runtime.enter();
                        drop(session);
                        return;
                    }
                    *ui.session.borrow_mut() = Some(Arc::new(session));
                    ui.with_conversation(|c| {
                        c.started();
                        Vec::new()
                    });
                    events
                }
                Err(e) => {
                    ui.with_conversation(|c| c.failed(&e.to_string()));
                    return;
                }
            };
            // `Events` is a plain channel; it needs no tokio reactor.
            while let Some(event) = events.recv().await {
                // With a session, a request too large to review is denied at once.
                let session = ui.session();
                ui.with_conversation(|c| match &session {
                    Some(session) => c.apply_with(event, &**session),
                    None => c.apply(event),
                });
            }
        });
    }

    /// Runs `f` on the conversation. If `f` made the session ready, the
    /// view-model hands out the launch `/usage` (once per conversation, only
    /// right then), and it is sent.
    fn with_conversation(self: &Rc<Self>, f: impl FnOnce(&mut Conversation) -> Vec<usize>) {
        let launch = match self.conversation.borrow_mut().as_mut() {
            Some(c) => {
                f(c);
                c.take_launch_usage()
            }
            None => return,
        };
        if let Some(out) = launch
            && self.session().is_some()
        {
            self.send_prompt(out);
        }
    }

    /// `Send`: the text it carries is what the input holds, then the composer
    /// decides whether it goes. Only an `Outgoing` it returns is sent.
    fn submit(self: &Rc<Self>, text: &str, via: Via) {
        let how = match via {
            Via::Button => How::SendButton,
            Via::Enter => How::EnterKey,
        };
        let can_send = self
            .conversation
            .borrow()
            .as_ref()
            .is_some_and(|c| c.can_send());
        let out = {
            let mut composer = self.composer.borrow_mut();
            composer.edited(text);
            composer.submit(how, can_send)
        };
        if let Some(out) = out {
            self.send_prompt(out);
        }
    }

    /// `CheckUsage`: the view-model decides, and its message is the constant
    /// `/usage`. The input is left as it is.
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
            return;
        };
        let text = out.into_text();
        // The launch `/usage` holds the input for at most `USAGE_LOCK`; then the
        // input opens and the turn keeps running.
        if sent == Sent::Hidden {
            let ui = self.clone();
            glib::timeout_add_local_once(USAGE_LOCK, move || {
                if let Some(c) = ui.conversation.borrow_mut().as_mut() {
                    c.release_usage_lock();
                }
            });
        }
        let turn = self
            .runtime
            .spawn(async move { session.prompt(&text).await });
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let result = turn
                .await
                .unwrap_or_else(|e| Err(Error::Protocol(e.to_string())));
            ui.with_conversation(|c| match sent {
                Sent::Hidden => c.usage_ended(result),
                Sent::Shown(_) => {
                    let changed = c.turn_ended(result);
                    // A refused message goes back in the input.
                    if let Some(text) = c.take_unsent() {
                        ui.composer.borrow_mut().restore(text);
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
        self.runtime.spawn(async move { session.cancel().await });
    }

    /// Denies what is pending, then stops the agent. A session still starting is
    /// dropped when it arrives.
    fn close_session(self: &Rc<Self>) {
        self.closed.set(true);
        let Some(session) = self.session.borrow_mut().take() else {
            return;
        };
        self.with_conversation(|c| c.close(&*session));
        // The session's drop hands the rest to the runtime.
        let _guard = self.runtime.enter();
        drop(session);
    }
}
