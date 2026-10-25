//! One session's conversation, as the window shows it.

use std::collections::HashMap;
use std::fmt;

use vexos_ai_core::{Agent, Decision, Error, Event, PermissionRequest, Session, ToolCall};

use crate::proposal::Proposal;

/// Where a click's answer goes. [`Session`] in the app; a recorder in tests.
pub trait Decider {
    fn decide(&self, request: u64, decision: Decision) -> Result<(), Error>;
}

impl Decider for Session {
    fn decide(&self, request: u64, decision: Decision) -> Result<(), Error> {
        Session::decide(self, request, decision)
    }
}

/// The agent's name in the UI. Not "Claude Code": the panel is not an
/// Anthropic product and must not look like one.
pub fn agent_label(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude",
        Agent::OpenCode => "OpenCode",
    }
}

pub fn choice_label(decision: Decision) -> &'static str {
    match decision {
        Decision::ApproveOnce => "Approve once",
        Decision::DenyOnce => "Deny once",
    }
}

pub fn mode_label(mode: &str) -> String {
    match mode {
        "default" | "build" => format!("{mode}: asks before changes"),
        "plan" => "plan: read-only".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Until the session has started and the core has confirmed its first
    /// mode: before that, prompts would be refused.
    Starting,
    Ready,
    /// A prompt turn is running.
    Busy,
    Ended(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    User(String),
    Agent(String),
    Tool(ToolCard),
    Approval(ApprovalCard),
    Notice(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCard {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Pending,
    Approved,
    Denied,
    /// The core answered it (denied or cancelled) before a click did: timeout,
    /// cancel, or an error.
    Withdrawn,
    /// Denied because the window or the session closed.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalCard {
    pub request: u64,
    pub title: String,
    pub proposal: Proposal,
    choices: Vec<Decision>,
    pub answer: Answer,
}

impl ApprovalCard {
    fn new(request: PermissionRequest) -> Self {
        ApprovalCard {
            request: request.id,
            proposal: Proposal::from_request(request.kind.as_deref(), &request.raw_input),
            title: request.title,
            choices: request.choices,
            answer: Answer::Pending,
        }
    }

    /// The buttons to show: the choices the core passed up, while pending.
    pub fn buttons(&self) -> Vec<(Decision, &'static str)> {
        if self.answer != Answer::Pending {
            return Vec::new();
        }
        self.choices.iter().map(|&d| (d, choice_label(d))).collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClickError {
    NoSuchCard,
    AlreadyAnswered,
    NotOffered,
    SessionEnded,
    /// The core refused the answer; the card is marked withdrawn.
    Core(String),
}

impl fmt::Display for ClickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClickError::NoSuchCard => f.write_str("no such request"),
            ClickError::AlreadyAnswered => f.write_str("already answered"),
            ClickError::NotOffered => f.write_str("that answer was not offered"),
            ClickError::SessionEnded => f.write_str("the session has ended"),
            ClickError::Core(why) => f.write_str(why),
        }
    }
}

pub struct Conversation {
    agent: Agent,
    mode: Option<String>,
    state: State,
    started: bool,
    items: Vec<Item>,
    tools: HashMap<String, usize>,
    cards: HashMap<u64, usize>,
    /// A message the core refused to send, for the input to offer again.
    unsent: Option<String>,
}

/// Shown instead of the core's `NotPinned` error.
pub const NOT_SENT: &str =
    "Not sent: the agent's mode was being reset. Your message is back in the input; send it again.";

impl Conversation {
    pub fn new(agent: Agent) -> Self {
        Conversation {
            agent,
            mode: None,
            state: State::Starting,
            started: false,
            items: Vec::new(),
            tools: HashMap::new(),
            cards: HashMap::new(),
            unsent: None,
        }
    }

    pub fn agent(&self) -> Agent {
        self.agent
    }

    /// The mode the core last confirmed, if any yet. Read-only.
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn can_send(&self) -> bool {
        self.state == State::Ready
    }

    /// The session started. Prompts open once the core confirms a mode too.
    pub fn started(&mut self) {
        self.started = true;
        self.open();
    }

    /// The core sends the first mode when it opens prompts.
    fn open(&mut self) {
        if self.state == State::Starting && self.started && self.mode.is_some() {
            self.state = State::Ready;
        }
    }

    /// The message the core refused, once.
    pub fn take_unsent(&mut self) -> Option<String> {
        self.unsent.take()
    }

    /// Takes one event from the core. Returns the indices of the items that
    /// changed or were added, in order. Never answers a request.
    pub fn apply(&mut self, event: Event) -> Vec<usize> {
        match event {
            Event::AgentText(text) => match self.items.last_mut() {
                Some(Item::Agent(last)) => {
                    last.push_str(&text);
                    vec![self.items.len() - 1]
                }
                _ => vec![self.push(Item::Agent(text))],
            },
            Event::ToolCall(call) => vec![self.tool(call)],
            Event::Permission(request) => {
                let id = request.id;
                let index = self.push(Item::Approval(ApprovalCard::new(request)));
                self.cards.insert(id, index);
                vec![index]
            }
            Event::Mode(mode) => {
                self.mode = Some(mode);
                self.open();
                Vec::new()
            }
            Event::Aborted(why) => self.end(format!("The session ended: {why}")),
            Event::Closed => self.end("The agent closed the session.".to_owned()),
        }
    }

    fn push(&mut self, item: Item) -> usize {
        self.items.push(item);
        self.items.len() - 1
    }

    /// Updates arrive by id, many without a title: a missing field keeps the
    /// value it had.
    fn tool(&mut self, call: ToolCall) -> usize {
        if let Some(&index) = self.tools.get(&call.id)
            && let Item::Tool(card) = &mut self.items[index]
        {
            card.title = call.title.or(card.title.take());
            card.kind = call.kind.or(card.kind.take());
            card.status = call.status.or(card.status.take());
            return index;
        }
        let id = call.id.clone();
        let index = self.push(Item::Tool(ToolCard {
            id: call.id,
            title: call.title,
            kind: call.kind,
            status: call.status,
        }));
        self.tools.insert(id, index);
        index
    }

    /// Marks every pending card and returns their indices.
    fn settle_pending(&mut self, answer: Answer) -> Vec<usize> {
        let mut changed: Vec<usize> = self
            .cards
            .values()
            .copied()
            .filter(|&i| {
                matches!(&self.items[i], Item::Approval(card) if card.answer == Answer::Pending)
            })
            .collect();
        changed.sort_unstable();
        for &i in &changed {
            if let Item::Approval(card) = &mut self.items[i] {
                card.answer = answer;
            }
        }
        changed
    }

    /// The core denies or drops whatever is pending when a session ends.
    fn end(&mut self, notice: String) -> Vec<usize> {
        if matches!(self.state, State::Ended(_)) {
            return Vec::new();
        }
        self.state = State::Ended(notice.clone());
        let mut changed = self.settle_pending(Answer::Closed);
        changed.push(self.push(Item::Notice(notice)));
        changed
    }

    /// The session could not start.
    pub fn failed(&mut self, why: &str) -> Vec<usize> {
        self.end(format!("The agent could not start: {why}"))
    }

    /// The user sent a message. The caller sends the prompt only on `Some`.
    pub fn send(&mut self, text: &str) -> Option<usize> {
        if !self.can_send() || text.trim().is_empty() {
            return None;
        }
        self.state = State::Busy;
        Some(self.push(Item::User(text.to_owned())))
    }

    /// The prompt turn finished, with its stop reason or an error.
    pub fn turn_ended(&mut self, result: Result<String, Error>) -> Vec<usize> {
        if self.state != State::Busy {
            return Vec::new();
        }
        self.state = State::Ready;
        match result {
            // A mode reset ran: nothing was sent. Not an error to show raw.
            Err(Error::NotPinned) => {
                self.unsent = self.items.iter().rev().find_map(|item| match item {
                    Item::User(text) => Some(text.clone()),
                    _ => None,
                });
                vec![self.push(Item::Notice(NOT_SENT.to_owned()))]
            }
            Ok(reason) if reason == "cancelled" => {
                vec![self.push(Item::Notice("Cancelled.".into()))]
            }
            Ok(_) => Vec::new(),
            Err(why) => vec![self.push(Item::Notice(format!("The request failed: {why}")))],
        }
    }

    /// The user cancelled the turn; the core answers pending requests
    /// `cancelled`.
    pub fn cancelled(&mut self) -> Vec<usize> {
        self.settle_pending(Answer::Withdrawn)
    }

    /// A click on a card's button: the only way a request gets answered.
    pub fn click(
        &mut self,
        request: u64,
        decision: Decision,
        decider: &dyn Decider,
    ) -> Result<usize, ClickError> {
        let index = *self.cards.get(&request).ok_or(ClickError::NoSuchCard)?;
        let ended = matches!(self.state, State::Ended(_));
        let Item::Approval(card) = &mut self.items[index] else {
            return Err(ClickError::NoSuchCard);
        };
        if card.answer != Answer::Pending {
            return Err(ClickError::AlreadyAnswered);
        }
        if ended {
            return Err(ClickError::SessionEnded);
        }
        if !card.choices.contains(&decision) {
            return Err(ClickError::NotOffered);
        }
        match decider.decide(request, decision) {
            Ok(()) => {
                card.answer = match decision {
                    Decision::ApproveOnce => Answer::Approved,
                    Decision::DenyOnce => Answer::Denied,
                };
                Ok(index)
            }
            Err(e) => {
                card.answer = Answer::Withdrawn;
                Err(ClickError::Core(e.to_string()))
            }
        }
    }

    /// The window or the session is closing: deny every pending request that
    /// offers "deny once". The caller then drops the session, which answers the
    /// rest. Nothing is ever approved here.
    pub fn close(&mut self, decider: &dyn Decider) -> Vec<usize> {
        let pending = self.settle_pending(Answer::Closed);
        for &i in &pending {
            if let Item::Approval(card) = &self.items[i]
                && card.choices.contains(&Decision::DenyOnce)
            {
                let _ = decider.decide(card.request, Decision::DenyOnce);
            }
        }
        if !matches!(self.state, State::Ended(_)) {
            self.state = State::Ended("Closed.".to_owned());
        }
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Recorder {
        calls: RefCell<Vec<(u64, Decision)>>,
        fail: bool,
    }

    impl Decider for Recorder {
        fn decide(&self, request: u64, decision: Decision) -> Result<(), Error> {
            self.calls.borrow_mut().push((request, decision));
            if self.fail {
                Err(Error::UnknownRequest)
            } else {
                Ok(())
            }
        }
    }

    impl Recorder {
        fn calls(&self) -> Vec<(u64, Decision)> {
            self.calls.borrow().clone()
        }
    }

    const BOTH: [Decision; 2] = [Decision::ApproveOnce, Decision::DenyOnce];

    fn request(id: u64, choices: &[Decision]) -> Event {
        Event::Permission(PermissionRequest {
            id,
            tool_call_id: Some(format!("t{id}")),
            title: "touch x".into(),
            kind: Some("execute".into()),
            raw_input: json!({ "command": "touch x" }),
            choices: choices.to_vec(),
        })
    }

    fn tool(id: &str, title: Option<&str>, kind: Option<&str>, status: Option<&str>) -> Event {
        Event::ToolCall(ToolCall {
            id: id.into(),
            title: title.map(Into::into),
            kind: kind.map(Into::into),
            status: status.map(Into::into),
        })
    }

    fn ready() -> Conversation {
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        c.apply(Event::Mode("default".into()));
        c
    }

    fn card(c: &Conversation, index: usize) -> &ApprovalCard {
        match &c.items()[index] {
            Item::Approval(card) => card,
            other => panic!("not a card: {other:?}"),
        }
    }

    // Fed by the core's events -----------------------------------------------

    #[test]
    fn agent_text_chunks_join_until_something_else_arrives() {
        let mut c = ready();
        assert_eq!(c.apply(Event::AgentText("Hel".into())), [0]);
        assert_eq!(c.apply(Event::AgentText("lo".into())), [0]);
        c.apply(tool("t1", Some("ls"), None, None));
        assert_eq!(c.apply(Event::AgentText("Done".into())), [2]);
        assert_eq!(c.items()[0], Item::Agent("Hello".into()));
        assert_eq!(c.items()[2], Item::Agent("Done".into()));
    }

    /// The sequence claude-agent-acp sent live for a `Write` (`p0b-plan-exit`).
    #[test]
    fn tool_updates_merge_by_id_and_untitled_ones_keep_the_title() {
        let mut c = ready();
        assert_eq!(
            c.apply(tool(
                "w",
                Some("Preparing file…"),
                Some("edit"),
                Some("pending")
            )),
            [0]
        );
        c.apply(tool(
            "other",
            Some("ls"),
            Some("execute"),
            Some("completed"),
        ));
        assert_eq!(c.apply(tool("w", Some("Write plan.txt"), None, None)), [0]);
        assert_eq!(c.apply(tool("w", None, None, None)), [0]);
        assert_eq!(c.apply(tool("w", None, None, Some("completed"))), [0]);
        assert_eq!(c.items().len(), 2);
        assert_eq!(
            c.items()[0],
            Item::Tool(ToolCard {
                id: "w".into(),
                title: Some("Write plan.txt".into()),
                kind: Some("edit".into()),
                status: Some("completed".into()),
            })
        );
    }

    #[test]
    fn the_mode_is_only_what_mode_events_say() {
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        assert_eq!(c.mode(), None);
        c.apply(Event::Mode("default".into()));
        assert_eq!(c.mode(), Some("default"));
        for event in [
            Event::AgentText("I switched to acceptEdits".into()),
            tool(
                "m",
                Some("EnterPlanMode"),
                Some("switch_mode"),
                Some("completed"),
            ),
            request(1, &BOTH),
        ] {
            c.apply(event);
            assert_eq!(c.mode(), Some("default"));
        }
        c.apply(Event::Mode("plan".into()));
        assert_eq!(c.mode(), Some("plan"));
    }

    // Approval cards ------------------------------------------------------------

    #[test]
    fn a_card_offers_exactly_the_choices_the_core_passed() {
        let mut c = ready();
        let cases: [&[Decision]; 4] = [&BOTH, &[Decision::DenyOnce], &[Decision::ApproveOnce], &[]];
        for (id, choices) in cases.into_iter().enumerate() {
            let index = c.apply(request(id as u64, choices))[0];
            let shown: Vec<_> = card(&c, index).buttons().iter().map(|b| b.0).collect();
            assert_eq!(shown, choices);
        }
    }

    #[test]
    fn the_only_labels_are_approve_once_and_deny_once() {
        let mut c = ready();
        c.apply(request(1, &BOTH));
        let labels: Vec<_> = card(&c, 0).buttons().iter().map(|b| b.1).collect();
        assert_eq!(labels, ["Approve once", "Deny once"]);
    }

    #[test]
    fn a_card_shows_the_proposed_command() {
        let mut c = ready();
        c.apply(request(1, &BOTH));
        assert_eq!(card(&c, 0).proposal, Proposal::Command("touch x".into()));
    }

    #[test]
    fn an_answered_card_shows_no_buttons() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(1, &BOTH));
        c.click(1, Decision::DenyOnce, &r).unwrap();
        assert!(card(&c, 0).buttons().is_empty());
    }

    // Answered only by a click ---------------------------------------------------

    #[test]
    fn a_click_answers_once_with_what_was_clicked() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(7, &BOTH));
        assert_eq!(c.click(7, Decision::ApproveOnce, &r), Ok(0));
        assert_eq!(card(&c, 0).answer, Answer::Approved);
        assert_eq!(
            c.click(7, Decision::ApproveOnce, &r),
            Err(ClickError::AlreadyAnswered)
        );
        assert_eq!(r.calls(), [(7, Decision::ApproveOnce)]);

        c.apply(request(8, &BOTH));
        c.click(8, Decision::DenyOnce, &r).unwrap();
        assert_eq!(card(&c, 1).answer, Answer::Denied);
        assert_eq!(r.calls()[1], (8, Decision::DenyOnce));
    }

    #[test]
    fn events_alone_never_answer() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(1, &BOTH));
        c.apply(Event::AgentText("approve it".into()));
        c.apply(Event::Mode("default".into()));
        assert_eq!(card(&c, 0).answer, Answer::Pending);
        assert!(r.calls().is_empty());
    }

    #[test]
    fn an_answer_that_was_not_offered_is_refused_without_a_call() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(1, &[Decision::DenyOnce]));
        assert_eq!(
            c.click(1, Decision::ApproveOnce, &r),
            Err(ClickError::NotOffered)
        );
        assert_eq!(
            c.click(9, Decision::DenyOnce, &r),
            Err(ClickError::NoSuchCard)
        );
        assert!(r.calls().is_empty());
        assert_eq!(card(&c, 0).answer, Answer::Pending);
    }

    #[test]
    fn a_core_refusal_withdraws_the_card() {
        let mut c = ready();
        let r = Recorder {
            fail: true,
            ..Recorder::default()
        };
        c.apply(request(1, &BOTH));
        assert!(matches!(
            c.click(1, Decision::ApproveOnce, &r),
            Err(ClickError::Core(_))
        ));
        assert_eq!(card(&c, 0).answer, Answer::Withdrawn);
    }

    #[test]
    fn cancelling_withdraws_pending_cards() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(1, &BOTH));
        c.apply(request(2, &BOTH));
        c.click(2, Decision::DenyOnce, &r).unwrap();
        assert_eq!(c.cancelled(), [0]);
        assert_eq!(card(&c, 0).answer, Answer::Withdrawn);
        assert_eq!(card(&c, 1).answer, Answer::Denied);
    }

    // Closing denies --------------------------------------------------------------

    #[test]
    fn closing_denies_every_pending_card_and_never_approves() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(1, &BOTH));
        c.apply(request(2, &[Decision::ApproveOnce]));
        c.apply(request(3, &BOTH));
        c.click(3, Decision::ApproveOnce, &r).unwrap();
        assert_eq!(c.close(&r), [0, 1]);
        assert_eq!(
            r.calls(),
            [(3, Decision::ApproveOnce), (1, Decision::DenyOnce)]
        );
        assert_eq!(card(&c, 0).answer, Answer::Closed);
        assert_eq!(card(&c, 1).answer, Answer::Closed);
        assert_eq!(card(&c, 2).answer, Answer::Approved);
        assert!(matches!(c.state(), State::Ended(_)));
        assert_eq!(
            c.click(2, Decision::ApproveOnce, &r),
            Err(ClickError::AlreadyAnswered)
        );
    }

    #[test]
    fn a_session_that_ends_closes_its_cards_and_refuses_clicks() {
        for end in [Event::Aborted("mode reset failed".into()), Event::Closed] {
            let (mut c, r) = (ready(), Recorder::default());
            c.apply(request(1, &BOTH));
            assert_eq!(c.apply(end), [0, 1]);
            assert_eq!(card(&c, 0).answer, Answer::Closed);
            assert!(matches!(c.items()[1], Item::Notice(_)));
            c.apply(request(2, &BOTH));
            assert_eq!(
                c.click(2, Decision::ApproveOnce, &r),
                Err(ClickError::SessionEnded)
            );
            assert!(r.calls().is_empty());
            assert!(!c.can_send());
        }
    }

    // Prompts -------------------------------------------------------------------

    #[test]
    fn one_prompt_at_a_time_and_only_while_ready() {
        let mut c = Conversation::new(Agent::OpenCode);
        assert_eq!(c.send("hi"), None);
        c.started();
        c.apply(Event::Mode("build".into()));
        assert_eq!(c.send("   "), None);
        assert_eq!(c.send("hi"), Some(0));
        assert_eq!(c.send("again"), None);
        assert_eq!(c.turn_ended(Ok("end_turn".into())), Vec::<usize>::new());
        assert_eq!(c.send("again"), Some(1));
        assert_eq!(c.turn_ended(Err(Error::Closed)), [2]);
        assert_eq!(
            c.items()[2],
            Item::Notice("The request failed: the agent closed the connection".into())
        );
        c.failed("gone");
        assert_eq!(c.send("more"), None);
    }

    /// The core opens prompts when it confirms the first mode; until then they
    /// would be refused, so the input stays off.
    #[test]
    fn the_input_waits_for_the_session_and_its_first_confirmed_mode() {
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        assert!(!c.can_send());
        assert_eq!(c.send("hi"), None);
        assert_eq!(*c.state(), State::Starting);
        c.apply(Event::Mode("default".into()));
        assert!(c.can_send());

        let mut c = Conversation::new(Agent::OpenCode);
        c.apply(Event::Mode("build".into()));
        assert!(!c.can_send());
        c.started();
        assert!(c.can_send());
    }

    #[test]
    fn a_refused_prompt_is_not_shown_raw_and_its_text_comes_back() {
        let mut c = ready();
        c.send("check the swap size").unwrap();
        assert_eq!(c.turn_ended(Err(Error::NotPinned)), [1]);
        assert_eq!(c.items()[1], Item::Notice(NOT_SENT.into()));
        assert!(!NOT_SENT.contains("NotPinned"));
        assert_eq!(c.take_unsent().as_deref(), Some("check the swap size"));
        assert_eq!(c.take_unsent(), None);
        assert!(c.can_send());
    }

    #[test]
    fn the_agent_is_never_called_claude_code() {
        assert_eq!(agent_label(Agent::Claude), "Claude");
        assert_eq!(agent_label(Agent::OpenCode), "OpenCode");
    }
}
