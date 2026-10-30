//! One session's conversation, as the window shows it.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use vexos_ai_core::{Agent, Decision, Error, Event, PermissionRequest, Session, ToolCall};

use std::time::{Duration, Instant};

use crate::composer::Outgoing;
use crate::proposal::Proposal;
use crate::usage::{Disabled, Shown, UsageButton, five_hour, tooltip};

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

/// The saved agent choice, relative to the user config directory. The same
/// file `vexos-ai pick` writes and `status --json` reports (docs/contract.md).
pub const AGENT_FILE: &str = "vexos/ai/agent";

/// Longest part of the choice file that is read.
const MAX_CHOICE_BYTES: u64 = 64;

/// The agent a choice file names: its first line, exactly `claude` or
/// `opencode`. No trimming and no case folding, as in the CLI.
pub fn parse_agent(bytes: &[u8]) -> Option<Agent> {
    let line = bytes.split(|&b| b == b'\n').next().unwrap_or_default();
    match line {
        b"claude" => Some(Agent::Claude),
        b"opencode" => Some(Agent::OpenCode),
        _ => None,
    }
}

/// The saved agent, read-only. A missing, unreadable, unknown or empty choice
/// is Claude. It returns the enum only: the stored text never reaches a
/// command line, it just selects one of the two fixed launch configurations.
pub fn saved_agent(path: &Path) -> Agent {
    let read = || -> Option<Agent> {
        // A FIFO or device could block the window's start.
        if !std::fs::metadata(path).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        File::open(path)
            .ok()?
            .take(MAX_CHOICE_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        parse_agent(&bytes)
    };
    read().unwrap_or(Agent::Claude)
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

/// What the empty conversation page says. Display only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Empty {
    Starting(&'static str),
    Ready(&'static str),
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
    /// The core's timeout denied it: nobody answered in time.
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalCard {
    pub request: u64,
    /// The tool call this request is about, when the agent said.
    pub tool_call: Option<String>,
    pub title: String,
    pub proposal: Proposal,
    choices: Vec<Decision>,
    pub answer: Answer,
}

impl ApprovalCard {
    fn new(request: PermissionRequest) -> Self {
        ApprovalCard {
            request: request.id,
            tool_call: request.tool_call_id,
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
    /// The latest approval card for each tool call.
    approvals: HashMap<String, usize>,
    /// A message the core refused to send, for the input to offer again.
    unsent: Option<String>,
    /// The panel's setting: `/usage` runs once when the session is ready.
    usage_on_launch: bool,
    /// Set by the ready transition and cleared by the next call: the launch
    /// `/usage` can be taken only right then.
    launch_due: bool,
    /// The launch `/usage` was handed out: the next [`Self::send`] of it is
    /// the hidden turn.
    hidden_due: bool,
    /// The hidden launch turn while it runs: `true` while it locks the input.
    hidden: Option<bool>,
    /// Agent text goes to the hidden turn's reply, not the transcript: from
    /// its start until a visible turn takes over after it ended.
    hidden_route: bool,
    hidden_reply: String,
    /// The last usage result; `None` before any.
    usage: Option<Shown>,
    /// Launch timing on a monotonic clock. Display only: never sent, logged
    /// or stored.
    requested: Option<Instant>,
    startup: Option<Startup>,
    built: Option<Instant>,
    mapped: Option<Instant>,
    first_frame: Option<Instant>,
    spawn: Option<Instant>,
    ready: Option<Instant>,
}

/// How long a process has been running, from the text of `/proc/self/stat`
/// and `/proc/uptime` (proc(5)): the start time is field 22, in clock ticks
/// since boot (USER_HZ, always 100 for user space), the uptime is in seconds,
/// both on the boot clock. Field 2 (the name) may hold spaces and `)`, so
/// fields are counted after the last `)`. `None` for anything unexpected.
pub fn since_process_start(stat: &str, uptime: &str) -> Option<Duration> {
    let after = stat.get(stat.rfind(')')? + 1..)?;
    let ticks: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let up: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    // /proc/uptime has two decimals, so this is a whole number of ticks. Round:
    // `0.29 * 100.0` is 28.999..., and a floor would lose a tick.
    // `try_from_secs_f64` refuses what is not a sane age: negative (started
    // "in the future"), NaN, infinite, huge.
    let up_ticks = (up * 100.0).round();
    Duration::try_from_secs_f64((up_ticks - ticks as f64) / 100.0).ok()
}

/// How the process got to its first window, for the launch timing: what ran
/// before `main` (the wrappers and the dynamic linker), and when `main`, GTK
/// and the stylesheet were ready. Only the first window of a process has one.
/// Display only: never sent, logged or stored.
#[derive(Clone, Copy, Debug, Default)]
pub struct Startup {
    pub before_main: Option<Duration>,
    pub main: Option<Instant>,
    pub gtk_ready: Option<Instant>,
    pub styled: Option<Instant>,
}

/// What [`Conversation::send`] did with a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// A user line was added at this index; end it with
    /// [`Conversation::turn_ended`].
    Shown(usize),
    /// The launch `/usage`: not in the transcript; end it with
    /// [`Conversation::usage_ended`].
    Hidden,
}

/// The hidden turn's reply is kept up to this many bytes for the parser.
const MAX_HIDDEN_REPLY: usize = 64 * 1024;

/// Shown instead of the core's `NotPinned` error.
pub const NOT_SENT: &str =
    "Not sent: the agent's mode was being reset. Your message is back in the input; send it again.";

/// Shown when `/usage` gets no text back.
pub const NO_REPLY: &str = "The agent finished without a reply.";

/// Shown when `/usage` was refused during a mode reset. Unlike typed text, it
/// is not put back in the input.
pub const USAGE_NOT_SENT: &str =
    "Not sent: the agent's mode was being reset. Ask for /usage again when you want it.";

/// The only text [`Conversation::check_usage`] sends, at launch or on a click
/// on the usage line: Claude Code's own usage report. A constant, so nothing
/// else can go out that way.
pub const USAGE_COMMAND: &str = "/usage";

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
            approvals: HashMap::new(),
            unsent: None,
            usage_on_launch: true,
            launch_due: false,
            hidden_due: false,
            hidden: None,
            hidden_route: false,
            hidden_reply: String::new(),
            usage: None,
            requested: None,
            startup: None,
            built: None,
            mapped: None,
            first_frame: None,
            spawn: None,
            ready: None,
        }
    }

    /// When the window was requested, for the launch timing.
    pub fn window_requested(mut self, at: Instant) -> Self {
        self.requested = Some(at);
        self
    }

    /// The process's own start-up, for the first window of a process.
    pub fn startup(mut self, startup: Startup) -> Self {
        self.startup = Some(startup);
        self
    }

    /// The window's widgets were built. Only the first call counts.
    pub fn built(&mut self, at: Instant) {
        self.built.get_or_insert(at);
    }

    /// The window was mapped (`present` ran). Only the first call counts.
    pub fn mapped(&mut self, at: Instant) {
        self.mapped.get_or_insert(at);
    }

    /// The window's first frame was painted. Only the first call counts.
    pub fn first_frame(&mut self, at: Instant) {
        self.first_frame.get_or_insert(at);
    }

    /// The agent's start was requested. Only the first call counts.
    pub fn spawn_requested(&mut self, at: Instant) {
        self.spawn.get_or_insert(at);
    }

    /// Where the time went, once the first frame or the session is there.
    /// Times are cumulative: from the process start for the first window of a
    /// process, else from the request. Display only.
    pub fn launch_summary(&self) -> Option<String> {
        let requested = self.requested?;
        if self.first_frame.is_none() && self.ready.is_none() {
            return None;
        }
        let startup = self.startup;
        let base = startup.and_then(|s| s.main).unwrap_or(requested);
        let before = startup
            .and_then(|s| s.before_main)
            .unwrap_or_default()
            .as_secs_f64();
        let secs = |at: Instant| before + at.saturating_duration_since(base).as_secs_f64();
        let mut parts = Vec::new();
        let mut mark = |label: &str, at: Option<Instant>| {
            if let Some(at) = at {
                parts.push(format!("{label} {:.2} s", secs(at)));
            }
        };
        if let Some(s) = startup {
            mark("main", s.main);
            mark("GTK ready", s.gtk_ready);
            mark("stylesheet", s.styled);
        }
        mark("window requested", Some(requested));
        mark("built", self.built);
        mark("mapped", self.mapped);
        mark("first frame", self.first_frame);
        mark("agent start requested", self.spawn);
        mark("ready", self.ready);
        let origin = match startup.and_then(|s| s.before_main.zip(s.main)) {
            Some(_) => "Since the process started",
            None if startup.is_some() => "Since main",
            None => "Since the window was requested, process already running",
        };
        Some(format!("{origin}: {}", parts.join(", ")))
    }

    /// The note above the input while the session starts, and the input is
    /// off. Starting a session sends nothing; a waiting draft stays unsent.
    pub fn starting_note(&self, draft_waiting: bool) -> Option<String> {
        if self.state != State::Starting {
            return None;
        }
        let mut note = format!(
            "Starting {}… The input opens when the session is ready.",
            agent_label(self.agent)
        );
        if draft_waiting {
            note.push_str(
                " A draft from a notification is waiting; nothing is sent until you click Send.",
            );
        }
        Some(note)
    }

    /// The panel's `usage-on-launch` setting (on by default).
    pub fn usage_on_launch(mut self, on: bool) -> Self {
        self.usage_on_launch = on;
        self
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

    /// The page shown instead of the list while there are no items: which
    /// agent is starting, or is ready. `None` once anything is in the
    /// conversation, and once the session has ended.
    pub fn empty_state(&self) -> Option<Empty> {
        if !self.items.is_empty() {
            return None;
        }
        let agent = agent_label(self.agent);
        match self.state {
            State::Starting => Some(Empty::Starting(agent)),
            State::Ready | State::Busy => Some(Empty::Ready(agent)),
            State::Ended(_) => None,
        }
    }

    /// A message can be sent: ready, and not locked by the launch `/usage`.
    pub fn can_send(&self) -> bool {
        self.state == State::Ready && !self.usage_locked()
    }

    /// A turn is in flight, visible or the hidden launch one: Cancel works.
    pub fn turn_running(&self) -> bool {
        self.state == State::Busy || self.hidden.is_some()
    }

    /// The launch `/usage` is running and holds the input.
    pub fn usage_locked(&self) -> bool {
        self.hidden == Some(true)
    }

    /// The usage line is shown: only Claude has the command.
    pub fn offers_usage_check(&self) -> bool {
        self.agent == Agent::Claude
    }

    /// Why a click on the usage line would do nothing now, if it would.
    pub fn usage_disabled(&self) -> Option<Disabled> {
        if !self.offers_usage_check() {
            Some(Disabled::OpenCode)
        } else if matches!(self.state, State::Starting | State::Ended(_)) {
            Some(Disabled::NotReady)
        } else if self.hidden.is_some() {
            Some(Disabled::Loading)
        } else if self.state == State::Busy {
            Some(Disabled::TurnRunning)
        } else if self.approval_waiting() {
            Some(Disabled::ApprovalWaiting)
        } else {
            None
        }
    }

    /// A click on the usage line works now: Claude, the mode confirmed, no
    /// turn running (the launch one included), and no approval waiting.
    pub fn can_check_usage(&self) -> bool {
        self.usage_disabled().is_none() && self.can_send()
    }

    /// [`USAGE_COMMAND`] as a message when [`Self::can_check_usage`], else
    /// `None`. It takes no text, so it can send nothing else; it is the only
    /// maker of an [`Outgoing`] besides [`crate::Composer::submit`]. Called by
    /// a click on the usage line, and once by [`Self::take_launch_usage`].
    /// The caller sends it through [`Self::send`] like any message.
    pub fn check_usage(&self) -> Option<Outgoing> {
        self.can_check_usage()
            .then(|| Outgoing(USAGE_COMMAND.to_owned()))
    }

    /// The launch `/usage`: [`Self::check_usage`]'s message, once per
    /// conversation, and only right after the call that made it ready (a
    /// Claude session, with the setting on). Any later call gets `None`,
    /// whatever happens: a mode reset, a refused send, or a failure. Sent
    /// through [`Self::send`], it becomes the hidden turn.
    pub fn take_launch_usage(&mut self) -> Option<Outgoing> {
        if !std::mem::take(&mut self.launch_due) {
            return None;
        }
        let out = self.check_usage()?;
        self.hidden_due = true;
        Some(out)
    }

    /// The launch turn has held the input for [`crate::USAGE_LOCK`]: the input
    /// opens; the turn keeps running and the line keeps saying "loading".
    pub fn release_usage_lock(&mut self) -> Vec<usize> {
        if self.hidden.is_some() {
            self.hidden = Some(false);
        }
        Vec::new()
    }

    /// The usage line as the window shows it, at `now` (a monotonic clock).
    pub fn usage_button(&self, now: Instant) -> UsageButton {
        let disabled = self.usage_disabled();
        let (wide, narrow) = self.shown().labels(now);
        UsageButton {
            visible: self.offers_usage_check(),
            enabled: disabled.is_none() && self.can_send(),
            wide,
            narrow,
            tooltip: tooltip(disabled),
        }
    }

    fn shown(&self) -> Shown {
        let launch_ahead =
            self.state == State::Starting && self.offers_usage_check() && self.usage_on_launch;
        if self.hidden.is_some() || launch_ahead {
            return Shown::Loading;
        }
        self.usage.clone().unwrap_or(Shown::Unknown)
    }

    /// The result of a `/usage` reply, or "unknown" when it doesn't parse.
    fn record_usage(&mut self, reply: Option<&str>) {
        self.usage = Some(match reply.and_then(five_hour) {
            Some(f) => Shown::Known(f, Instant::now()),
            None => Shown::Unknown,
        });
    }

    fn approval_waiting(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, Item::Approval(card) if card.answer == Answer::Pending))
    }

    /// The session started. Prompts open once the core confirms a mode too.
    pub fn started(&mut self) {
        self.launch_due = false;
        self.started = true;
        self.open();
    }

    /// The core sends the first mode when it opens prompts. This is the one
    /// ready transition (nothing goes back to `Starting`), and the only place
    /// the launch `/usage` is armed.
    fn open(&mut self) {
        if self.state == State::Starting && self.started && self.mode.is_some() {
            self.state = State::Ready;
            self.ready.get_or_insert_with(Instant::now);
            if self.offers_usage_check() && self.usage_on_launch {
                self.launch_due = true;
            }
        }
    }

    /// The message the core refused, once.
    pub fn take_unsent(&mut self) -> Option<String> {
        self.unsent.take()
    }

    /// Takes one event from the core. Returns the indices of the items that
    /// changed or were added, in order. Never answers a request.
    pub fn apply(&mut self, event: Event) -> Vec<usize> {
        self.launch_due = false;
        match event {
            // The hidden turn's reply goes to the parser only. Text arriving
            // after it ended (before a visible turn) is still its reply.
            Event::AgentText(text) if self.hidden_route => {
                if self.hidden_reply.len() + text.len() <= MAX_HIDDEN_REPLY {
                    self.hidden_reply.push_str(&text);
                }
                if self.hidden.is_none() {
                    let reply = std::mem::take(&mut self.hidden_reply);
                    if five_hour(&reply).is_some() {
                        self.record_usage(Some(&reply));
                    }
                    self.hidden_reply = reply;
                }
                Vec::new()
            }
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
                let tool_call = request.tool_call_id.clone();
                let index = self.push(Item::Approval(ApprovalCard::new(request)));
                self.cards.insert(id, index);
                if let Some(tool_call) = tool_call {
                    self.approvals.insert(tool_call, index);
                }
                vec![index]
            }
            Event::TimedOut(request) => self.timed_out(request),
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

    /// The status a tool card shows. A call that failed because its approval
    /// was not given says "denied"; any other failure still says "failed". A
    /// managed-policy deny sends no request, so it can't be told apart from a
    /// failure and says "failed".
    pub fn tool_status<'a>(&self, tool: &'a ToolCard) -> &'a str {
        let status = tool.status.as_deref().unwrap_or("pending");
        let not_approved = self.approvals.get(&tool.id).is_some_and(|&i| {
            matches!(&self.items[i], Item::Approval(card)
                if !matches!(card.answer, Answer::Pending | Answer::Approved))
        });
        if status == "failed" && not_approved {
            "denied"
        } else {
            status
        }
    }

    /// The items to redraw when request `request`'s card changes: the card,
    /// and the tool card whose status depends on it.
    pub fn card_rows(&self, request: u64) -> Vec<usize> {
        self.cards
            .get(&request)
            .map(|&index| self.rows(index))
            .unwrap_or_default()
    }

    fn rows(&self, card: usize) -> Vec<usize> {
        let mut rows = vec![card];
        if let Item::Approval(ApprovalCard {
            tool_call: Some(id),
            ..
        }) = &self.items[card]
            && let Some(&tool) = self.tools.get(id)
        {
            rows.push(tool);
        }
        rows
    }

    /// The core denied `request` on its timeout. That answer is final,
    /// whatever a click said in the meantime.
    fn timed_out(&mut self, request: u64) -> Vec<usize> {
        let Some(&index) = self.cards.get(&request) else {
            return Vec::new();
        };
        if let Item::Approval(card) = &mut self.items[index] {
            card.answer = Answer::TimedOut;
        }
        self.rows(index)
    }

    /// Marks every pending card and returns the items to redraw.
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
        let mut rows: Vec<usize> = changed.into_iter().flat_map(|i| self.rows(i)).collect();
        rows.sort_unstable();
        rows.dedup();
        rows
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

    /// A message goes out. The caller sends the prompt only on `Some`. Only
    /// [`crate::Composer::submit`] and [`Self::check_usage`] make an
    /// [`Outgoing`]. The launch `/usage`, right after
    /// [`Self::take_launch_usage`], is the hidden turn; anything else adds a
    /// user line.
    pub fn send(&mut self, out: &Outgoing) -> Option<Sent> {
        let hidden = std::mem::take(&mut self.hidden_due) && out.text() == USAGE_COMMAND;
        self.launch_due = false;
        if !self.can_send() || out.text().trim().is_empty() {
            return None;
        }
        if hidden {
            self.hidden = Some(true);
            self.hidden_route = true;
            self.hidden_reply.clear();
            return Some(Sent::Hidden);
        }
        // Text goes to the hidden turn until it ends, then to this one; the
        // agent answers prompts in order.
        if self.hidden.is_none() {
            self.hidden_route = false;
        }
        self.state = State::Busy;
        Some(Sent::Shown(self.push(Item::User(out.text().to_owned()))))
    }

    /// The hidden launch turn finished. Its reply goes to the parser and
    /// nowhere else; a failure, a cancel or a refusal makes the line
    /// "unknown". Nothing is retried, and the transcript doesn't change.
    pub fn usage_ended(&mut self, result: Result<String, Error>) -> Vec<usize> {
        if self.hidden.take().is_none() {
            return Vec::new();
        }
        if self.state == State::Busy {
            // A visible turn was sent meanwhile: the text is its now.
            self.hidden_route = false;
        }
        let reply = std::mem::take(&mut self.hidden_reply);
        let done = matches!(&result, Ok(reason) if reason != "cancelled");
        self.record_usage(done.then_some(reply.as_str()));
        if self.hidden_route {
            self.hidden_reply = reply;
        }
        Vec::new()
    }

    /// The prompt turn finished, with its stop reason or an error.
    pub fn turn_ended(&mut self, result: Result<String, Error>) -> Vec<usize> {
        if self.state != State::Busy {
            return Vec::new();
        }
        self.state = State::Ready;
        // A `/usage` the user clicked or typed: its reply refreshes the line
        // when it parses; a failure leaves the line as it was. No other
        // turn's reply reaches the parser.
        if self.last_sent().map(str::trim) == Some(USAGE_COMMAND)
            && matches!(&result, Ok(reason) if reason != "cancelled")
        {
            let reply = self.visible_reply();
            if five_hour(&reply).is_some() {
                self.record_usage(Some(&reply));
            }
        }
        match result {
            // `/usage` is a command, not the user's words: it never goes into
            // the input, where it would end up in front of what they typed.
            Err(Error::NotPinned) if self.last_sent() == Some(USAGE_COMMAND) => {
                vec![self.push(Item::Notice(USAGE_NOT_SENT.to_owned()))]
            }
            // A mode reset ran: nothing was sent. Not an error to show raw.
            Err(Error::NotPinned) => {
                self.unsent = self.last_sent().map(str::to_owned);
                vec![self.push(Item::Notice(NOT_SENT.to_owned()))]
            }
            Ok(reason) if reason == "cancelled" => {
                vec![self.push(Item::Notice("Cancelled.".into()))]
            }
            // An empty answer to `/usage` is said plainly, not left silent.
            // Never retried.
            Ok(_) if self.last_sent() == Some(USAGE_COMMAND) && !self.replied() => {
                vec![self.push(Item::Notice(NO_REPLY.to_owned()))]
            }
            Ok(_) => Vec::new(),
            Err(why) => vec![self.push(Item::Notice(format!("The request failed: {why}")))],
        }
    }

    /// The user's last message.
    fn last_sent(&self) -> Option<&str> {
        self.items.iter().rev().find_map(|item| match item {
            Item::User(text) => Some(text.as_str()),
            _ => None,
        })
    }

    /// The agent's text since the user's last message.
    fn visible_reply(&self) -> String {
        let mut parts: Vec<&str> = self
            .items
            .iter()
            .rev()
            .take_while(|item| !matches!(item, Item::User(_)))
            .filter_map(|item| match item {
                Item::Agent(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        parts.reverse();
        parts.join("\n")
    }

    /// The agent wrote some text since the user's last message.
    fn replied(&self) -> bool {
        self.items
            .iter()
            .rev()
            .take_while(|item| !matches!(item, Item::User(_)))
            .any(|item| matches!(item, Item::Agent(text) if !text.trim().is_empty()))
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
    use crate::usage::{CLICK_TOOLTIP, USAGE_LOCK};
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

    #[test]
    fn the_empty_page_names_the_agent_and_goes_with_the_first_item() {
        let mut c = Conversation::new(Agent::OpenCode);
        assert_eq!(c.empty_state(), Some(Empty::Starting("OpenCode")));
        c.started();
        assert_eq!(c.empty_state(), Some(Empty::Starting("OpenCode")));
        c.apply(Event::Mode("build".into()));
        assert_eq!(c.empty_state(), Some(Empty::Ready("OpenCode")));
        assert_eq!(
            Conversation::new(Agent::Claude).empty_state(),
            Some(Empty::Starting("Claude"))
        );
        c.apply(Event::AgentText("hi".into()));
        assert_eq!(c.empty_state(), None);
    }

    #[test]
    fn the_empty_page_is_gone_once_the_session_closed_even_with_no_items() {
        let mut c = ready();
        assert!(c.items().is_empty() && c.empty_state().is_some());
        c.close(&Recorder::default());
        assert!(matches!(c.state(), State::Ended(_)));
        assert!(c.items().is_empty());
        assert_eq!(c.empty_state(), None);
    }

    #[test]
    fn the_empty_page_is_gone_after_a_failure_or_a_notice() {
        let mut c = Conversation::new(Agent::Claude);
        c.failed("no agent");
        assert_eq!(c.empty_state(), None);
        let mut c = ready();
        assert!(c.empty_state().is_some());
        c.apply(request(1, &BOTH));
        assert_eq!(c.empty_state(), None);
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

    // Denied, not failed -----------------------------------------------------------

    fn tool_card(c: &Conversation, index: usize) -> &ToolCard {
        match &c.items()[index] {
            Item::Tool(card) => card,
            other => panic!("not a tool card: {other:?}"),
        }
    }

    /// A tool call `t1` asking for permission, answered by `answer`, then
    /// reported `failed`: what its tool card says.
    fn status_after(answer: impl FnOnce(&mut Conversation, &Recorder)) -> String {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(tool(
            "t1",
            Some("touch x"),
            Some("execute"),
            Some("pending"),
        ));
        c.apply(request(1, &BOTH));
        answer(&mut c, &r);
        c.apply(tool("t1", None, None, Some("failed")));
        c.tool_status(tool_card(&c, 0)).to_owned()
    }

    #[test]
    fn a_call_that_failed_after_its_approval_was_not_given_says_denied() {
        let deny = |c: &mut Conversation, r: &Recorder| {
            c.click(1, Decision::DenyOnce, r).unwrap();
        };
        assert_eq!(status_after(deny), "denied");
        assert_eq!(
            status_after(|c, _| {
                c.apply(Event::TimedOut(1));
            }),
            "denied"
        );
        assert_eq!(
            status_after(|c, _| {
                c.cancelled();
            }),
            "denied"
        );
        assert_eq!(
            status_after(|c, r| {
                c.close(r);
            }),
            "denied"
        );
    }

    #[test]
    fn a_genuine_failure_still_says_failed() {
        let approve = |c: &mut Conversation, r: &Recorder| {
            c.click(1, Decision::ApproveOnce, r).unwrap();
        };
        assert_eq!(status_after(approve), "failed");
        assert_eq!(status_after(|_, _| {}), "failed");

        // No request at all, as with a managed-policy deny.
        let mut c = ready();
        c.apply(tool(
            "t2",
            Some("nixos-rebuild switch"),
            None,
            Some("failed"),
        ));
        assert_eq!(c.tool_status(tool_card(&c, 0)), "failed");
        // A denied request about another call changes nothing here.
        let r = Recorder::default();
        c.apply(request(1, &BOTH));
        c.click(1, Decision::DenyOnce, &r).unwrap();
        assert_eq!(c.tool_status(tool_card(&c, 0)), "failed");
    }

    #[test]
    fn other_statuses_are_shown_as_they_are() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(tool("t1", None, None, None));
        c.apply(request(1, &BOTH));
        c.click(1, Decision::DenyOnce, &r).unwrap();
        assert_eq!(c.tool_status(tool_card(&c, 0)), "pending");
        c.apply(tool("t1", None, None, Some("completed")));
        assert_eq!(c.tool_status(tool_card(&c, 0)), "completed");
    }

    #[test]
    fn answering_a_card_also_redraws_its_tool_card() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(tool("t1", None, None, Some("failed")));
        c.apply(tool("t9", None, None, None));
        c.apply(request(1, &BOTH));
        c.apply(request(2, &BOTH));
        assert_eq!(c.card_rows(1), [2, 0]);
        assert_eq!(c.card_rows(7), Vec::<usize>::new());
        assert_eq!(c.cancelled(), [0, 2, 3]);
        assert_eq!(c.apply(Event::TimedOut(1)), [2, 0]);
        c.click(2, Decision::DenyOnce, &r).unwrap_err();
    }

    // Timed out ---------------------------------------------------------------------

    #[test]
    fn a_request_the_core_timed_out_says_so_and_offers_nothing() {
        let mut c = ready();
        c.apply(request(1, &BOTH));
        c.apply(request(2, &BOTH));
        c.apply(Event::TimedOut(1));
        assert_eq!(card(&c, 0).answer, Answer::TimedOut);
        assert!(card(&c, 0).buttons().is_empty());
        assert_eq!(card(&c, 1).answer, Answer::Pending);
        let r = Recorder::default();
        assert_eq!(
            c.click(1, Decision::ApproveOnce, &r),
            Err(ClickError::AlreadyAnswered)
        );
        assert!(r.calls().is_empty());
    }

    /// A click that reached the core after its timeout fired was not counted:
    /// the core denied the request.
    #[test]
    fn a_timeout_overrides_a_click_that_came_too_late() {
        let (mut c, r) = (ready(), Recorder::default());
        c.apply(request(1, &BOTH));
        c.click(1, Decision::ApproveOnce, &r).unwrap();
        c.apply(Event::TimedOut(1));
        assert_eq!(card(&c, 0).answer, Answer::TimedOut);
    }

    #[test]
    fn a_timeout_for_an_unknown_request_changes_nothing() {
        let mut c = ready();
        c.apply(request(1, &BOTH));
        assert_eq!(c.apply(Event::TimedOut(5)), Vec::<usize>::new());
        assert_eq!(card(&c, 0).answer, Answer::Pending);
    }

    // Literal approval text ----------------------------------------------------------

    #[test]
    fn approval_text_is_kept_literal() {
        let mut c = ready();
        let command = "echo **x** <span foreground=\"red\">`y`</span> [a](javascript:z)";
        c.apply(Event::Permission(PermissionRequest {
            id: 1,
            tool_call_id: None,
            title: "# **title**".into(),
            kind: Some("execute".into()),
            raw_input: json!({ "command": command }),
            choices: BOTH.to_vec(),
        }));
        assert_eq!(card(&c, 0).title, "# **title**");
        assert_eq!(card(&c, 0).proposal, Proposal::Command(command.into()));
    }

    // Prompts -------------------------------------------------------------------

    #[test]
    fn one_prompt_at_a_time_and_only_while_ready() {
        let mut c = Conversation::new(Agent::OpenCode);
        assert_eq!(c.send(&Outgoing("hi".into())), None);
        c.started();
        c.apply(Event::Mode("build".into()));
        assert_eq!(c.send(&Outgoing("   ".into())), None);
        assert_eq!(c.send(&Outgoing("hi".into())), Some(Sent::Shown(0)));
        assert_eq!(c.send(&Outgoing("again".into())), None);
        assert_eq!(c.turn_ended(Ok("end_turn".into())), Vec::<usize>::new());
        assert_eq!(c.send(&Outgoing("again".into())), Some(Sent::Shown(1)));
        assert_eq!(c.turn_ended(Err(Error::Closed)), [2]);
        assert_eq!(
            c.items()[2],
            Item::Notice("The request failed: the agent closed the connection".into())
        );
        c.failed("gone");
        assert_eq!(c.send(&Outgoing("more".into())), None);
    }

    // `check_usage`: the click on the usage line ---------------------------------

    /// The action's whole surface, by type: no argument but the conversation,
    /// read-only, so it can't take text or change anything; it can only hand
    /// back the constant.
    #[test]
    fn check_usage_takes_nothing_but_the_conversation() {
        let _: fn(&Conversation) -> Option<Outgoing> = Conversation::check_usage;
        assert_eq!(USAGE_COMMAND, "/usage");
    }

    #[test]
    fn check_usage_sends_exactly_slash_usage_through_the_normal_path() {
        let mut c = ready();
        assert!(c.offers_usage_check());
        let out = c.check_usage().expect("enabled");
        assert_eq!(out.text(), "/usage");
        assert_eq!(c.send(&out), Some(Sent::Shown(0)));
        assert_eq!(c.items()[0], Item::User("/usage".into()));
        assert_eq!(*c.state(), State::Busy);
        // The reply is ordinary agent text in the transcript (and the line's
        // parser reads it too; see the tests below).
        c.apply(Event::AgentText(
            "## Usage\n**5-hour limit** — **12%**".into(),
        ));
        assert_eq!(c.turn_ended(Ok("end_turn".into())), Vec::<usize>::new());
        assert_eq!(
            c.items()[1],
            Item::Agent("## Usage\n**5-hour limit** — **12%**".into())
        );
    }

    #[test]
    fn check_usage_is_not_offered_for_opencode() {
        let mut c = Conversation::new(Agent::OpenCode);
        c.started();
        c.apply(Event::Mode("build".into()));
        assert!(c.can_send());
        assert!(!c.offers_usage_check());
        assert!(!c.can_check_usage());
        assert_eq!(c.check_usage(), None);
    }

    #[test]
    fn check_usage_waits_for_the_session_and_its_confirmed_mode() {
        let mut c = Conversation::new(Agent::Claude);
        assert_eq!(c.check_usage(), None);
        c.started();
        assert_eq!(c.check_usage(), None);
        c.apply(Event::Mode("default".into()));
        assert!(c.check_usage().is_some());

        let mut c = Conversation::new(Agent::Claude);
        c.apply(Event::Mode("plan".into()));
        assert_eq!(c.check_usage(), None);
        c.started();
        assert!(c.check_usage().is_some());
    }

    #[test]
    fn check_usage_is_off_while_a_turn_runs() {
        let mut c = ready();
        c.send(&Outgoing("how big is swap?".into())).unwrap();
        assert!(!c.can_check_usage());
        assert_eq!(c.check_usage(), None);
        c.apply(Event::AgentText("4 GiB".into()));
        c.turn_ended(Ok("end_turn".into()));
        assert!(c.check_usage().is_some());
    }

    #[test]
    fn check_usage_is_off_while_an_approval_waits() {
        let mut c = ready();
        c.send(&Outgoing("touch x".into())).unwrap();
        c.apply(request(1, &BOTH));
        // The turn ended but the card is still waiting for a click.
        c.turn_ended(Ok("end_turn".into()));
        assert!(c.can_send());
        assert!(!c.can_check_usage());
        assert_eq!(c.check_usage(), None);
        c.click(1, Decision::DenyOnce, &Recorder::default())
            .unwrap();
        assert!(c.check_usage().is_some());
    }

    #[test]
    fn check_usage_is_off_once_the_session_ends() {
        let mut c = ready();
        c.apply(Event::Closed);
        assert_eq!(c.check_usage(), None);
        let mut c = Conversation::new(Agent::Claude);
        c.failed("gone");
        assert_eq!(c.check_usage(), None);
    }

    #[test]
    fn a_disabled_click_changes_nothing() {
        let mut c = ready();
        c.send(&Outgoing("hi".into())).unwrap();
        let (items, state) = (c.items().to_vec(), c.state().clone());
        assert_eq!(c.check_usage(), None);
        assert_eq!((c.items(), c.state()), (items.as_slice(), &state));
    }

    /// Nothing but a click sends it: starting, every kind of event and the end
    /// of a turn leave no message behind.
    #[test]
    fn check_usage_is_never_sent_by_itself() {
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        for event in [
            Event::Mode("default".into()),
            Event::AgentText("Run /usage to see your limits.".into()),
            tool("t1", Some("ls"), Some("read"), Some("completed")),
            request(2, &BOTH),
            Event::TimedOut(2),
            Event::Mode("plan".into()),
        ] {
            c.apply(event);
        }
        c.turn_ended(Ok("end_turn".into()));
        assert!(!c.items().iter().any(|item| matches!(item, Item::User(_))));
        assert_eq!(*c.state(), State::Ready);
    }

    #[test]
    fn an_empty_reply_is_said_plainly_and_not_retried() {
        let mut c = ready();
        c.send(&c.check_usage().unwrap()).unwrap();
        assert_eq!(c.turn_ended(Ok("end_turn".into())), [1]);
        assert_eq!(c.items()[1], Item::Notice(NO_REPLY.into()));
        // Blank text is no reply either.
        c.send(&c.check_usage().unwrap()).unwrap();
        c.apply(Event::AgentText(" \n".into()));
        assert_eq!(c.turn_ended(Ok("end_turn".into())), [4]);
        assert_eq!(c.items()[4], Item::Notice(NO_REPLY.into()));
        // Nothing was sent again by itself.
        let sent = c.items().iter().filter(|i| matches!(i, Item::User(_)));
        assert_eq!(sent.count(), 2);
        assert_eq!(*c.state(), State::Ready);
    }

    #[test]
    fn an_empty_reply_to_typed_text_adds_nothing() {
        let mut c = ready();
        c.send(&Outgoing("touch x".into())).unwrap();
        c.apply(request(1, &BOTH));
        assert_eq!(c.turn_ended(Ok("end_turn".into())), Vec::<usize>::new());
        assert!(!c.items().contains(&Item::Notice(NO_REPLY.into())));
    }

    #[test]
    fn a_refused_usage_check_never_goes_into_the_input() {
        let mut c = ready();
        c.send(&c.check_usage().unwrap()).unwrap();
        assert_eq!(c.turn_ended(Err(Error::NotPinned)), [1]);
        assert_eq!(c.items()[1], Item::Notice(USAGE_NOT_SENT.into()));
        assert_eq!(c.take_unsent(), None);
        // Not retried, and the button works again.
        assert_eq!(c.items().len(), 2);
        assert!(c.check_usage().is_some());
        // Typed text still comes back as before.
        c.send(&Outgoing("check the swap size".into())).unwrap();
        c.turn_ended(Err(Error::NotPinned));
        assert_eq!(c.take_unsent().as_deref(), Some("check the swap size"));
    }

    #[test]
    fn an_error_is_shown_as_text_and_not_retried() {
        let mut c = ready();
        c.send(&c.check_usage().unwrap()).unwrap();
        assert_eq!(c.turn_ended(Err(Error::Closed)), [1]);
        assert_eq!(
            c.items()[1],
            Item::Notice("The request failed: the agent closed the connection".into())
        );
        assert_eq!(c.items().len(), 2);
        assert_eq!(*c.state(), State::Ready);
    }

    // The launch `/usage` and the usage line ---------------------------------------

    const LINE: &str = "**5-hour limit** — **25%** · Resets Oct 27, 1:40 PM CDT";
    const WEEKLY: &str = "**Weekly · all models** — **49%** · Resets Oct 30, 6:00 PM CDT";

    fn report(five: &str) -> String {
        format!(
            "## Usage\n\n### Limits\n\n{five}\n\n{WEEKLY}\n\n| Cost | Active |\n|:--|:--|\n| $0.12 | 1m |"
        )
    }

    /// A Claude session through its ready transition, with the setting.
    fn launching(on: bool) -> Conversation {
        let mut c = Conversation::new(Agent::Claude).usage_on_launch(on);
        c.started();
        c.apply(Event::Mode("default".into()));
        c
    }

    /// The launch `/usage` sent, as the hidden turn.
    fn hidden() -> Conversation {
        let mut c = launching(true);
        let out = c.take_launch_usage().expect("launch");
        assert_eq!(c.send(&out), Some(Sent::Hidden));
        c
    }

    fn label(c: &Conversation) -> String {
        c.usage_button(Instant::now()).wide
    }

    fn users(c: &Conversation) -> usize {
        c.items()
            .iter()
            .filter(|i| matches!(i, Item::User(_)))
            .count()
    }

    #[test]
    fn launch_sends_the_constant_once_right_after_ready() {
        let mut c = launching(true);
        let out = c.take_launch_usage().expect("launch");
        assert_eq!(out.text(), "/usage");
        assert_eq!(c.take_launch_usage(), None);
        // The ready transition in the other order arms it too.
        let mut c = Conversation::new(Agent::Claude);
        c.apply(Event::Mode("default".into()));
        c.started();
        assert!(c.take_launch_usage().is_some());
    }

    #[test]
    fn launch_is_only_right_at_the_ready_transition() {
        // Any call in between, and the moment has passed.
        let mut c = launching(true);
        c.apply(Event::AgentText("hello".into()));
        assert_eq!(c.take_launch_usage(), None);
        // Before ready: nothing.
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        assert_eq!(c.take_launch_usage(), None);
    }

    #[test]
    fn no_launch_for_opencode_or_with_the_setting_off() {
        let mut c = Conversation::new(Agent::OpenCode);
        c.started();
        c.apply(Event::Mode("build".into()));
        assert_eq!(c.take_launch_usage(), None);
        assert!(!c.usage_button(Instant::now()).visible);

        let mut c = launching(false);
        assert_eq!(c.take_launch_usage(), None);
        assert_eq!(label(&c), "Usage: unknown");
        // A click still works.
        assert!(c.check_usage().is_some());
    }

    #[test]
    fn no_launch_again_after_a_reset_a_refusal_or_a_failure() {
        let mut c = hidden();
        assert_eq!(c.usage_ended(Err(Error::NotPinned)), Vec::<usize>::new());
        // The panel asks right after every call: nothing to send again.
        assert_eq!(c.take_launch_usage(), None);
        assert_eq!(label(&c), "Usage: unknown");
        // A mode reset and a new mode report: no second launch.
        c.apply(Event::Mode("plan".into()));
        c.apply(Event::Mode("default".into()));
        assert_eq!(c.take_launch_usage(), None);
        c.started();
        assert_eq!(c.take_launch_usage(), None);
        assert_eq!(users(&c), 0);
    }

    #[test]
    fn no_launch_while_an_approval_waits() {
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        c.apply(request(1, &BOTH));
        c.apply(Event::Mode("default".into()));
        assert_eq!(c.take_launch_usage(), None);
        c.click(1, Decision::DenyOnce, &Recorder::default())
            .unwrap();
        assert_eq!(c.take_launch_usage(), None);
    }

    #[test]
    fn a_launch_message_sent_later_is_an_ordinary_one() {
        // Taken, then something else happened first: no hidden turn later.
        let mut c = launching(true);
        let out = c.take_launch_usage().unwrap();
        c.send(&Outgoing("hi".into())).unwrap();
        c.apply(Event::AgentText("hello".into()));
        c.turn_ended(Ok("end_turn".into()));
        assert_eq!(c.send(&out), Some(Sent::Shown(2)));
    }

    #[test]
    fn the_hidden_reply_goes_only_to_the_parser() {
        let mut c = hidden();
        assert_eq!(label(&c), "Usage: loading");
        assert_eq!(c.apply(Event::AgentText(report(LINE))), Vec::<usize>::new());
        assert_eq!(c.usage_ended(Ok("end_turn".into())), Vec::<usize>::new());
        assert!(c.items().is_empty());
        let b = c.usage_button(Instant::now());
        assert_eq!(
            b.wide,
            "5-hour 25% · resets Oct 27, 1:40 PM CDT · updated just now"
        );
        assert_eq!(b.narrow, "25%");
        assert!(b.enabled);
    }

    #[test]
    fn a_click_shows_the_full_reply_and_refreshes_the_line() {
        let mut c = hidden();
        c.apply(Event::AgentText(report(LINE)));
        c.usage_ended(Ok("end_turn".into()));
        let out = c.check_usage().unwrap();
        assert_eq!(c.send(&out), Some(Sent::Shown(0)));
        let full = report("**5-hour limit** — **30%** · Resets Oct 27, 6:40 PM CDT");
        c.apply(Event::AgentText(full.clone()));
        c.turn_ended(Ok("end_turn".into()));
        assert_eq!(c.items(), [Item::User("/usage".into()), Item::Agent(full)]);
        assert!(label(&c).starts_with("5-hour 30% · resets Oct 27, 6:40 PM CDT"));
    }

    #[test]
    fn a_failed_or_unparsable_click_keeps_the_line() {
        let mut c = hidden();
        c.apply(Event::AgentText(LINE.into()));
        c.usage_ended(Ok("end_turn".into()));
        for result in [
            Ok("end_turn".to_owned()),
            Ok("cancelled".to_owned()),
            Err(Error::Closed),
        ] {
            c.send(&c.check_usage().unwrap()).unwrap();
            c.apply(Event::AgentText("Usage is not available right now.".into()));
            c.turn_ended(result);
            assert!(
                label(&c).starts_with("5-hour 25%"),
                "{label}",
                label = label(&c)
            );
        }
    }

    #[test]
    fn a_typed_usage_refreshes_the_line_too() {
        let mut c = launching(false);
        c.send(&Outgoing("/usage".into())).unwrap();
        c.apply(Event::AgentText(report(LINE)));
        c.turn_ended(Ok("end_turn".into()));
        assert!(label(&c).starts_with("5-hour 25%"));
        assert_eq!(users(&c), 1);
    }

    #[test]
    fn no_other_turns_reply_reaches_the_parser() {
        let mut c = launching(false);
        c.send(&Outgoing("what does /usage show?".into())).unwrap();
        c.apply(Event::AgentText(format!("It shows:\n{LINE}")));
        c.turn_ended(Ok("end_turn".into()));
        assert_eq!(label(&c), "Usage: unknown");
        // After a known value, another turn's reply doesn't change it.
        let mut c = hidden();
        c.apply(Event::AgentText(LINE.into()));
        c.usage_ended(Ok("end_turn".into()));
        c.send(&Outgoing("hi".into())).unwrap();
        c.apply(Event::AgentText(
            "**5-hour limit** — **99%** · Resets Jan 1, 1:00 AM UTC".into(),
        ));
        c.turn_ended(Ok("end_turn".into()));
        assert!(label(&c).starts_with("5-hour 25%"));
    }

    #[test]
    fn an_empty_failed_or_cancelled_launch_says_unknown() {
        for result in [
            Ok("end_turn".to_owned()),
            Ok("cancelled".to_owned()),
            Err(Error::Closed),
        ] {
            let mut c = hidden();
            c.usage_ended(result);
            assert_eq!(label(&c), "Usage: unknown");
            assert!(c.items().is_empty());
        }
        // Claude Code's own fallback text, not the adapter's report.
        let mut c = hidden();
        c.apply(Event::AgentText(
            "Current session\n  25% used\n  Resets 1:40pm".into(),
        ));
        c.usage_ended(Ok("end_turn".into()));
        assert_eq!(label(&c), "Usage: unknown");
    }

    #[test]
    fn late_hidden_text_still_goes_to_the_parser_until_a_visible_turn() {
        let mut c = hidden();
        c.usage_ended(Ok("end_turn".into()));
        assert_eq!(label(&c), "Usage: unknown");
        c.apply(Event::AgentText(LINE.into()));
        assert!(c.items().is_empty());
        assert!(label(&c).starts_with("5-hour 25%"));
        c.send(&Outgoing("hi".into())).unwrap();
        c.apply(Event::AgentText("hello".into()));
        assert_eq!(c.items()[1], Item::Agent("hello".into()));
    }

    #[test]
    fn the_launch_turn_locks_the_input_until_it_ends_or_is_released() {
        let mut c = hidden();
        assert!(c.usage_locked() && c.turn_running());
        assert!(!c.can_send());
        assert_eq!(c.check_usage(), None);
        assert_eq!(c.usage_disabled(), Some(Disabled::Loading));
        // The panel's 15 s timer releases it; the turn keeps running.
        assert_eq!(USAGE_LOCK, std::time::Duration::from_secs(15));
        c.release_usage_lock();
        assert!(!c.usage_locked() && c.turn_running());
        assert!(c.can_send());
        assert_eq!(label(&c), "Usage: loading");
        assert_eq!(c.check_usage(), None);
        // A visible turn meanwhile: the launch reply still goes to the parser.
        c.send(&Outgoing("hi".into())).unwrap();
        c.apply(Event::AgentText(LINE.into()));
        c.usage_ended(Ok("end_turn".into()));
        assert!(label(&c).starts_with("5-hour 25%"));
        c.apply(Event::AgentText("hello".into()));
        assert_eq!(
            c.items(),
            [Item::User("hi".into()), Item::Agent("hello".into())]
        );
        // Release after the end does nothing.
        c.release_usage_lock();
        assert!(!c.usage_locked());
    }

    #[test]
    fn button_states_and_their_reasons() {
        let now = Instant::now();
        let mut c = Conversation::new(Agent::Claude);
        let b = c.usage_button(now);
        assert!(b.visible && !b.enabled);
        assert_eq!(
            (b.wide.as_str(), b.tooltip),
            ("Usage: loading", tooltip(Some(Disabled::NotReady)))
        );
        c.started();
        c.apply(Event::Mode("default".into()));
        let out = c.take_launch_usage().unwrap();
        c.send(&out);
        assert_eq!(
            c.usage_button(now).tooltip,
            tooltip(Some(Disabled::Loading))
        );
        c.usage_ended(Ok("end_turn".into()));
        let b = c.usage_button(now);
        assert!(b.enabled);
        assert_eq!(b.tooltip, CLICK_TOOLTIP);
        c.send(&Outgoing("touch x".into())).unwrap();
        assert_eq!(c.usage_disabled(), Some(Disabled::TurnRunning));
        assert!(!c.usage_button(now).enabled);
        c.apply(request(1, &BOTH));
        c.turn_ended(Ok("end_turn".into()));
        assert_eq!(c.usage_disabled(), Some(Disabled::ApprovalWaiting));
        assert!(!c.usage_button(now).enabled);
        c.apply(Event::Closed);
        assert_eq!(c.usage_disabled(), Some(Disabled::NotReady));
        let c = Conversation::new(Agent::OpenCode);
        assert_eq!(c.usage_disabled(), Some(Disabled::OpenCode));
        assert!(!c.usage_button(now).visible);
    }

    /// The real adapter's reset text has U+202F before "PM". The label gets
    /// the validated text, with an ASCII space, and nothing else.
    #[test]
    fn a_no_break_space_in_the_reply_reaches_the_label_as_an_ascii_space() {
        let mut c = hidden();
        c.apply(Event::AgentText(report(
            "**5-hour limit** — **25%** · Resets Oct 27, 1:40\u{202f}PM CDT",
        )));
        c.usage_ended(Ok("end_turn".into()));
        let b = c.usage_button(Instant::now());
        assert_eq!(
            b.wide,
            "5-hour 25% · resets Oct 27, 1:40 PM CDT · updated just now"
        );
        assert!(!b.wide.contains('\u{202f}'));
        assert_eq!(b.narrow, "25%");
        assert!(c.items().is_empty());
        // Mapped, but something else invalid: the whole value is unknown.
        let mut c = hidden();
        c.apply(Event::AgentText(
            "**5-hour limit** — **25%** · Resets 1:40\u{202f}PM GMT+2".into(),
        ));
        c.usage_ended(Ok("end_turn".into()));
        assert_eq!(label(&c), "Usage: unknown");
    }

    #[test]
    fn the_line_ages_from_a_monotonic_clock() {
        let mut c = hidden();
        c.apply(Event::AgentText(LINE.into()));
        c.usage_ended(Ok("end_turn".into()));
        let later = Instant::now() + std::time::Duration::from_secs(3 * 60 + 5);
        assert_eq!(
            c.usage_button(later).wide,
            "5-hour 25% · resets Oct 27, 1:40 PM CDT · updated 3 min ago"
        );
    }

    /// The core opens prompts when it confirms the first mode; until then they
    /// would be refused, so the input stays off.
    #[test]
    fn the_input_waits_for_the_session_and_its_first_confirmed_mode() {
        let mut c = Conversation::new(Agent::Claude);
        c.started();
        assert!(!c.can_send());
        assert_eq!(c.send(&Outgoing("hi".into())), None);
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
        c.send(&Outgoing("check the swap size".into())).unwrap();
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

    // The agent from settings --------------------------------------------------

    use serde_json::Value;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/agent-choice-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fixture(name: &str, bytes: &[u8]) -> PathBuf {
        let path = fixture_dir().join(name);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_choice_is_exactly_one_of_two_values() {
        assert_eq!(parse_agent(b"claude"), Some(Agent::Claude));
        assert_eq!(parse_agent(b"claude\n"), Some(Agent::Claude));
        assert_eq!(parse_agent(b"opencode"), Some(Agent::OpenCode));
        assert_eq!(parse_agent(b"opencode\n"), Some(Agent::OpenCode));
        // The first line only, as `head -n 1`.
        assert_eq!(parse_agent(b"opencode\nclaude"), Some(Agent::OpenCode));
        assert_eq!(parse_agent(b"claude\nrm -rf"), Some(Agent::Claude));
        for bad in [
            &b"Claude"[..],
            b"CLAUDE",
            b"codex",
            b" claude",
            b"claude ",
            b"claude\r",
            b"claude\r\n",
            b"claude\0",
            b"\x1bclaude",
            b"opencode\t",
            b"open code",
            b"",
            b"\n",
            b"\nclaude",
            b"\xff\xfe",
        ] {
            assert_eq!(parse_agent(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_missing_empty_unknown_or_unreadable_choice_is_claude() {
        let missing = fixture_dir().join("missing-choice");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(saved_agent(&missing), Agent::Claude);
        assert!(!missing.exists(), "reading never creates the file");
        assert_eq!(saved_agent(&fixture("empty", b"")), Agent::Claude);
        assert_eq!(saved_agent(&fixture("unknown", b"codex\n")), Agent::Claude);
        assert_eq!(
            saved_agent(&fixture("control", b"opencode\r\n")),
            Agent::Claude
        );
        assert_eq!(saved_agent(&fixture_dir()), Agent::Claude, "a directory");
        assert_eq!(saved_agent(&fixture("claude", b"claude\n")), Agent::Claude);
        assert_eq!(
            saved_agent(&fixture("opencode", b"opencode\n")),
            Agent::OpenCode
        );
    }

    #[test]
    fn a_huge_choice_file_is_read_in_part_and_judged_by_its_first_line() {
        let mut junk = vec![b'x'; 1 << 20];
        assert_eq!(saved_agent(&fixture("huge-junk", &junk)), Agent::Claude);
        let mut open = b"opencode\n".to_vec();
        open.append(&mut junk);
        assert_eq!(
            saved_agent(&fixture("huge-opencode", &open)),
            Agent::OpenCode
        );
        // No line break within the limit: not a value.
        let long = [b"opencode".as_slice(), &[b' '; 100]].concat();
        assert_eq!(saved_agent(&fixture("long-line", &long)), Agent::Claude);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_in_place_of_the_choice_does_not_block() {
        let path = fixture_dir().join("fifo");
        let _ = std::fs::remove_file(&path);
        // A Windows drive under WSL cannot hold a FIFO: set
        // VEXOS_ALLOW_NO_FIFO=1 there. Anywhere else a failure fails the test,
        // so the Nix sandbox cannot pass it without running it.
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made && std::env::var_os("VEXOS_ALLOW_NO_FIFO").is_some() {
            eprintln!("skipped: no FIFO on this filesystem");
            return;
        }
        assert!(made, "mkfifo failed");
        let (tx, rx) = std::sync::mpsc::channel();
        let probe = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(saved_agent(&probe));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(5));
        let _ = std::fs::remove_file(&path);
        assert_eq!(got, Ok(Agent::Claude));
    }

    #[test]
    fn the_stored_value_never_reaches_a_command() {
        let known = [Agent::Claude.command(), Agent::OpenCode.command()];
        for hostile in [
            "opencode --yolo",
            "claude; rm -rf ~",
            "$(id)",
            "claude\nopencode --yolo",
            "--dangerously-skip-permissions",
        ] {
            let agent = saved_agent(&fixture("hostile", hostile.as_bytes()));
            let command = agent.command();
            assert!(known.contains(&command), "{hostile}");
            let line = format!("{} {:?} {:?}", command.program, command.args, command.env);
            for word in ["rm -rf", "$(id)", "yolo", "dangerously"] {
                assert!(!line.contains(word), "{hostile}: {line}");
            }
        }
    }

    #[test]
    fn reading_the_choice_changes_nothing() {
        let path = fixture("readonly", b"opencode\n");
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        for _ in 0..3 {
            saved_agent(&path);
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"opencode\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn an_opencode_session_from_the_file_hides_the_usage_button() {
        let agent = saved_agent(&fixture("opencode-session", b"opencode\n"));
        assert_eq!(agent, Agent::OpenCode);
        let mut c = Conversation::new(agent);
        assert!(!c.usage_button(Instant::now()).visible);
        c.started();
        c.apply(Event::Mode("build".into()));
        assert!(!c.usage_button(Instant::now()).visible);
        assert!(c.take_launch_usage().is_none());
    }

    #[test]
    fn starting_keeps_the_input_off_and_says_so() {
        let mut c = Conversation::new(Agent::Claude);
        assert!(!c.can_send());
        assert!(!c.usage_button(Instant::now()).enabled);
        let note = c.starting_note(false).unwrap();
        assert!(note.contains("Starting Claude"));
        assert!(!note.contains("draft"));
        assert!(
            c.starting_note(true)
                .unwrap()
                .contains("nothing is sent until you click Send")
        );
        // The session started, but no mode is confirmed yet: still off.
        c.started();
        assert!(!c.can_send());
        assert!(c.starting_note(false).is_some());
        c.apply(Event::Mode("default".into()));
        assert!(c.can_send());
        assert_eq!(c.starting_note(true), None);
        let open = Conversation::new(Agent::OpenCode)
            .starting_note(false)
            .unwrap();
        assert!(open.contains("Starting OpenCode"));
    }

    #[test]
    fn launch_times_are_recorded_once_and_only_from_the_first_frame_or_ready() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut c = Conversation::new(Agent::Claude).window_requested(t0);
        c.spawn_requested(ms(10));
        c.spawn_requested(ms(9000));
        c.built(ms(20));
        c.built(ms(9000));
        c.mapped(ms(30));
        c.mapped(ms(9000));
        // Nothing to show before the first frame or ready.
        assert_eq!(c.launch_summary(), None);
        c.started();
        assert_eq!(c.launch_summary(), None);
        c.first_frame(ms(450));
        c.first_frame(ms(9000));
        let shown = c.launch_summary().unwrap();
        assert_eq!(
            shown,
            "Since the window was requested, process already running: window requested 0.00 s, \
             built 0.02 s, mapped 0.03 s, first frame 0.45 s, agent start requested 0.01 s"
        );
        c.apply(Event::Mode("default".into()));
        let first = c.launch_summary().unwrap();
        assert!(first.starts_with(&shown), "{first}");
        assert!(first.contains(", ready "), "{first}");
        c.apply(Event::Mode("plan".into()));
        assert_eq!(c.launch_summary().unwrap(), first);
        assert_eq!(Conversation::new(Agent::Claude).launch_summary(), None);
        // Without a request there is nothing to measure from.
        let mut bare = Conversation::new(Agent::Claude);
        bare.first_frame(ms(1));
        assert_eq!(bare.launch_summary(), None);
    }

    #[test]
    fn the_first_window_of_a_process_is_timed_from_the_process_start() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let startup = Startup {
            before_main: Some(Duration::from_millis(1500)),
            main: Some(t0),
            gtk_ready: Some(ms(200)),
            styled: Some(ms(210)),
        };
        let mut c = Conversation::new(Agent::Claude)
            .window_requested(ms(300))
            .startup(startup);
        c.built(ms(350));
        c.mapped(ms(360));
        c.first_frame(ms(900));
        assert_eq!(
            c.launch_summary().unwrap(),
            "Since the process started: main 1.50 s, GTK ready 1.70 s, stylesheet 1.71 s, \
             window requested 1.80 s, built 1.85 s, mapped 1.86 s, first frame 2.40 s"
        );
        // A process age that could not be read: counted from main.
        let unknown = Startup {
            before_main: None,
            ..startup
        };
        let mut c = Conversation::new(Agent::Claude)
            .window_requested(ms(300))
            .startup(unknown);
        c.first_frame(ms(900));
        let text = c.launch_summary().unwrap();
        assert!(
            text.starts_with("Since main: main 0.00 s, GTK ready 0.20 s"),
            "{text}"
        );
    }

    #[test]
    fn process_age_comes_from_stat_and_uptime() {
        // Field 22 is 1234 ticks; the 20th field after the name.
        let fields = |name: &str| {
            format!(
                "42 ({name}) S 1 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 5 0 1234 1000 200 18446744073709551615 1 2 3"
            )
        };
        let age = |name: &str, up: &str| since_process_start(&fields(name), up);
        assert_eq!(
            age("panel", "15.34 60.0"),
            Some(Duration::from_millis(3000))
        );
        // Spaces, `)` and digits in the name do not shift the fields.
        for name in [
            "a b",
            "x) S 1 2 3",
            "((",
            ") (",
            "9 8 7 6 5 4 3 2 1 0 9 8 7 6 5 4 3 2 1 0",
        ] {
            assert_eq!(
                age(name, "15.34 60.0"),
                Some(Duration::from_millis(3000)),
                "{name}"
            );
        }
        // Just started; and started "in the future" (a bad clock) is refused.
        assert_eq!(age("p", "12.34 1.0"), Some(Duration::ZERO));
        // Uptimes whose product with 100 falls just below a whole number must
        // not lose a tick.
        let at_zero = fields("p").replace(" 1234 ", " 0 ");
        for (up, ms) in [("0.29", 290), ("0.57", 570), ("1.13", 1130), ("4.35", 4350)] {
            let want = Some(Duration::from_millis(ms));
            assert_eq!(since_process_start(&at_zero, up), want, "{up}");
        }
        assert_eq!(age("p", "12.33 1.0"), None);
        // Short, empty, non-numeric, negative, NaN and huge input.
        for (stat, up) in [
            ("", "1.0"),
            ("42 (p) S 1", "1.0"),
            ("no parens at all 1 2 3", "1.0"),
            ("42 (p", "1.0"),
        ] {
            assert_eq!(since_process_start(stat, up), None, "{stat:?}");
        }
        assert_eq!(age("p", ""), None);
        assert_eq!(age("p", "soon"), None);
        assert_eq!(age("p", "-5.0"), None);
        assert_eq!(age("p", "NaN"), None);
        assert_eq!(age("p", "inf"), None);
        let bad = fields("p").replace(" 1234 ", " twelve ");
        assert_eq!(since_process_start(&bad, "60.0"), None);
        let huge = fields("p").replace(" 1234 ", " 99999999999999999999999 ");
        assert_eq!(since_process_start(&huge, "60.0"), None);
    }

    /// Timing against a fake agent on an in-memory pipe, as the core's tests
    /// drive one. Run with `--nocapture` to see the numbers.
    #[tokio::test]
    async fn launch_times_against_a_fake_agent() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let requested = Instant::now();
        let mut c = Conversation::new(Agent::Claude).window_requested(requested);
        let (core, agent_end) = tokio::io::duplex(1 << 16);
        let (r, w) = tokio::io::split(core);
        c.spawn_requested(Instant::now());
        let config = vexos_ai_core::Config::new(std::env::temp_dir().join("vexos-ai-test"));
        let start = tokio::spawn(Session::connect(Agent::Claude, config, r, w));
        let (r, mut w) = tokio::io::split(agent_end);
        let mut lines = BufReader::new(r).lines();
        let mut answers = [
            (
                "initialize",
                json!({ "protocolVersion": 1, "agentCapabilities": {} }),
            ),
            (
                "session/new",
                json!({ "sessionId": "s1", "modes": { "currentModeId": "default",
                    "availableModes": [{ "id": "default" }, { "id": "plan" }] } }),
            ),
            ("session/set_mode", json!({})),
        ]
        .into_iter();
        let handshake = async {
            for (method, result) in answers.by_ref() {
                let line = lines.next_line().await.unwrap().unwrap();
                let msg: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(msg["method"], method);
                let out = json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result });
                w.write_all(format!("{out}\n").as_bytes()).await.unwrap();
            }
        };
        let (_, started) = tokio::join!(handshake, start);
        let (_session, mut events) = started.unwrap().unwrap();
        c.started();
        while c.state() == &State::Starting {
            let event = events.recv().await.expect("a mode event");
            c.apply(event);
        }
        println!(
            "launch against the fake agent: {}",
            c.launch_summary().expect("ready")
        );
        assert!(c.can_send());
    }
}
