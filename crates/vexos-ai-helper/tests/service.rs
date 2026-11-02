//! The session behind the front door, driven call by call against a fake agent
//! on an in-memory pipe. No D-Bus here: the calls are the typed ones the gate
//! hands over.
//!
//! GLib objects belong to the thread that made them and the service lives on
//! the default main context, so every scenario runs in turn on this one test's
//! thread, as in `tests/bus.rs`.

use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use glib::MainContext;
use serde_json::json;
use tokio::runtime::Runtime;
use vexos_ai_core::Agent;
use vexos_ai_helper::bus::{Handler, HandlerError, Reply};
use vexos_ai_helper::gate::{Call, Choice, Placement, Via};
use vexos_ai_helper::service::{Service, Settings};
use vexos_ai_viewmodel::{Answer, Item, State};

#[path = "../testing/fake_agent.rs"]
mod fake_agent;

use fake_agent::{Cmd, FakeStarter};

fn run<F: Future>(future: F) -> F::Output {
    MainContext::default().block_on(future)
}

/// Waits, running the main context, until `cond` holds.
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if cond() {
            return;
        }
        glib::timeout_future(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Long enough for anything that was going to happen to have happened.
async fn settle() {
    glib::timeout_future(Duration::from_millis(300)).await;
}

struct Rig {
    runtime: Option<Runtime>,
    service: Rc<Service>,
    fake: FakeStarter,
}

impl Rig {
    fn new(usage_on_launch: bool) -> Rig {
        Rig::with(usage_on_launch, |_| {})
    }

    fn with(usage_on_launch: bool, set: impl FnOnce(&mut FakeStarter)) -> Rig {
        let runtime = Runtime::new().unwrap();
        let mut fake = FakeStarter::new(runtime.handle().clone());
        set(&mut fake);
        let service = Rc::new(Service::new(
            runtime.handle().clone(),
            Box::new(fake.clone()),
            Settings {
                agent: Agent::Claude,
                usage_on_launch,
            },
        ));
        Rig {
            runtime: Some(runtime),
            service,
            fake,
        }
    }

    fn call(&self, call: Call) -> Result<Reply, HandlerError> {
        self.service.handle(call)
    }

    fn ready(&self) -> bool {
        self.service
            .conversation()
            .as_ref()
            .is_some_and(|c| c.can_send())
    }

    async fn opened(&self) {
        assert_eq!(self.call(Call::Open), Ok(Reply::Done));
        until("the session to be ready", || self.ready()).await;
    }

    fn send(&self, text: &str) -> Result<Reply, HandlerError> {
        self.call(Call::Send {
            text: text.to_owned(),
            via: Via::Button,
        })
    }

    fn cards(&self) -> Vec<(u64, Answer)> {
        self.service
            .conversation()
            .as_ref()
            .map(|c| {
                c.items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Approval(card) => Some((card.request, card.answer)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.service.end();
        drop(self.runtime.take());
    }
}

fn denied(outcome: &Option<serde_json::Value>) -> bool {
    match outcome {
        Some(o) => {
            o == &json!({ "outcome": "selected", "optionId": "no" })
                || o == &json!({ "outcome": "cancelled" })
        }
        None => false,
    }
}

// Opening.

fn open_starts_one_session_and_gets_it_ready_and_a_second_open_starts_none() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        assert_eq!(rig.fake.starts(), 1);
        let mode = rig
            .service
            .conversation()
            .as_ref()
            .and_then(|c| c.mode().map(str::to_owned));
        assert_eq!(mode.as_deref(), Some("default"));
        assert_eq!(rig.call(Call::Open), Ok(Reply::Done));
        settle().await;
        assert_eq!(rig.fake.starts(), 1);
        // Starting is not sending.
        assert!(rig.fake.seen.prompts().is_empty());
    });
}

fn the_launch_usage_goes_out_once_when_the_setting_is_on() {
    let rig = Rig::new(true);
    run(async {
        assert_eq!(rig.call(Call::Open), Ok(Reply::Done));
        until("the launch /usage", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.fake.seen.prompts(), ["/usage"]);
        rig.fake.tell(Cmd::EndTurn);
        until("the input to open", || rig.ready()).await;
        settle().await;
        assert_eq!(rig.fake.seen.prompts(), ["/usage"]);
    });
}

fn no_launch_usage_when_the_setting_is_off() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        settle().await;
        assert!(rig.fake.seen.prompts().is_empty());
    });
}

fn a_start_that_fails_leaves_a_conversation_that_cannot_send() {
    let rig = Rig::with(false, |fake| fake.fail = true);
    run(async {
        assert_eq!(rig.call(Call::Open), Ok(Reply::Done));
        until("the failure", || {
            matches!(
                rig.service
                    .conversation()
                    .as_ref()
                    .map(|c| c.state().clone()),
                Some(State::Ended(_))
            )
        })
        .await;
        assert_eq!(rig.send("hello"), Ok(Reply::Done));
        assert_eq!(rig.call(Call::Open), Ok(Reply::Done));
        settle().await;
        assert!(rig.fake.seen.prompts().is_empty());
        assert_eq!(rig.fake.starts(), 1);
    });
}

// Sending.

fn send_puts_exactly_the_typed_text_in_one_prompt_and_empties_the_input() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        let text = "  hello\n\n  world \u{1F600}  ";
        assert_eq!(rig.send(text), Ok(Reply::Done));
        until("the prompt", || !rig.fake.seen.prompts().is_empty()).await;
        settle().await;
        assert_eq!(rig.fake.seen.prompts(), [text]);
        assert_eq!(rig.service.composer().text(), "");
    });
}

fn send_sends_its_own_text_not_the_last_edited_one() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        assert_eq!(rig.call(Call::Edited("old".into())), Ok(Reply::Done));
        assert_eq!(rig.send("new"), Ok(Reply::Done));
        until("the prompt", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.fake.seen.prompts(), ["new"]);
    });
}

fn enter_sends_like_the_button_when_there_is_no_draft() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        let call = Call::Send {
            text: "go".into(),
            via: Via::Enter,
        };
        assert_eq!(rig.call(call), Ok(Reply::Done));
        until("the prompt", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.fake.seen.prompts(), ["go"]);
    });
}

fn nothing_is_sent_when_it_may_not_be_and_the_text_stays_in_the_input() {
    let rig = Rig::new(false);
    run(async {
        // Before there is a session at all.
        assert_eq!(rig.send("too early"), Ok(Reply::Done));
        assert_eq!(rig.service.composer().text(), "too early");
        rig.opened().await;
        // Blank text.
        assert_eq!(rig.send("   \n"), Ok(Reply::Done));
        // While a turn runs.
        assert_eq!(rig.send("first"), Ok(Reply::Done));
        until("the first prompt", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.send("second"), Ok(Reply::Done));
        settle().await;
        assert_eq!(rig.fake.seen.prompts(), ["first"]);
        assert_eq!(rig.service.composer().text(), "second");
        // The turn ends: the input is free again.
        rig.fake.tell(Cmd::EndTurn);
        until("the turn to end", || rig.ready()).await;
        assert_eq!(rig.send("second"), Ok(Reply::Done));
        until("the second prompt", || rig.fake.seen.prompts().len() == 2).await;
        assert_eq!(rig.fake.seen.prompts(), ["first", "second"]);
    });
}

fn check_usage_sends_the_fixed_text_only_when_it_may() {
    let rig = Rig::new(false);
    run(async {
        // No session: nothing, and no failure.
        assert_eq!(rig.call(Call::CheckUsage), Ok(Reply::Done));
        rig.opened().await;
        assert_eq!(rig.call(Call::CheckUsage), Ok(Reply::Done));
        until("the prompt", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.fake.seen.prompts(), ["/usage"]);
        // The turn is running: a second click does nothing.
        assert_eq!(rig.call(Call::CheckUsage), Ok(Reply::Done));
        settle().await;
        assert_eq!(rig.fake.seen.prompts(), ["/usage"]);
    });
}

fn cancel_reaches_the_agent() {
    let rig = Rig::new(false);
    run(async {
        // No session: nothing, and no failure.
        assert_eq!(rig.call(Call::Cancel), Ok(Reply::Done));
        rig.opened().await;
        assert_eq!(rig.send("work"), Ok(Reply::Done));
        until("the prompt", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.fake.seen.count("session/cancel"), 0);
        assert_eq!(rig.call(Call::Cancel), Ok(Reply::Done));
        until("the cancel", || rig.fake.seen.count("session/cancel") == 1).await;
    });
}

fn the_draft_calls_change_nothing_while_there_is_no_draft() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        assert_eq!(rig.call(Call::Edited("typed".into())), Ok(Reply::Done));
        assert_eq!(
            rig.call(Call::DraftChoice(Placement::Replace)),
            Ok(Reply::Done)
        );
        assert_eq!(rig.call(Call::DraftDiscard), Ok(Reply::Done));
        assert_eq!(rig.service.composer().text(), "typed");
    });
}

fn a_lost_edit_is_remembered_until_the_next_edit_and_a_send_makes_the_input_known() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        assert_eq!(rig.call(Call::Edited("typed".into())), Ok(Reply::Done));
        assert_eq!(rig.service.composer().text(), "typed");
        assert!(!rig.service.composer().unknown());
        assert_eq!(rig.call(Call::EditLost), Ok(Reply::Done));
        assert!(rig.service.composer().unknown());
        // The next edit says what the input holds.
        assert_eq!(rig.call(Call::Edited("typed".into())), Ok(Reply::Done));
        assert!(!rig.service.composer().unknown());
        assert_eq!(rig.call(Call::EditLost), Ok(Reply::Done));
        // The Send carries the whole text, so it makes the input known again.
        assert_eq!(rig.send("typed"), Ok(Reply::Done));
        assert!(!rig.service.composer().unknown());
        until("the prompt", || !rig.fake.seen.prompts().is_empty()).await;
        assert_eq!(rig.fake.seen.prompts(), ["typed"]);
    });
}

fn a_message_the_core_refuses_goes_back_in_the_input() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        // The agent changes its mode; until the core's reset is acknowledged,
        // it refuses prompts.
        rig.fake.tell(Cmd::DriftMode);
        until("the mode reset", || {
            rig.fake.seen.count("session/set_mode") == 2
        })
        .await;
        assert_eq!(rig.send("keep me"), Ok(Reply::Done));
        until("the message to come back", || {
            rig.service.composer().text() == "keep me"
        })
        .await;
        assert!(rig.fake.seen.prompts().is_empty());
        // Once the reset holds, the same text goes.
        rig.fake.tell(Cmd::AckReset);
        until("the input to open", || rig.ready()).await;
        for _ in 0..200 {
            assert_eq!(rig.send("keep me"), Ok(Reply::Done));
            if !rig.fake.seen.prompts().is_empty() {
                break;
            }
            glib::timeout_future(Duration::from_millis(20)).await;
        }
        assert_eq!(rig.fake.seen.prompts(), ["keep me"]);
    });
}

// Approvals.

fn a_request_waits_unanswered_and_decide_and_shown_do_not_reach_it_until_close_denies_it() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        rig.fake.tell(Cmd::Ask {
            rpc_id: 100,
            command: "touch x".into(),
        });
        until("the card", || rig.cards() == [(1, Answer::Pending)]).await;
        // The core numbers requests itself: this is its first.
        let decide = Call::Decide {
            generation: 1,
            request: 1,
            choice: Choice::ApproveOnce,
        };
        assert_eq!(rig.call(decide), Err(HandlerError::NotReady));
        let shown = Call::Shown {
            generation: 1,
            request: 1,
        };
        assert_eq!(rig.call(shown), Err(HandlerError::NotReady));
        settle().await;
        assert_eq!(rig.fake.seen.answer(100), None);
        assert_eq!(rig.cards(), [(1, Answer::Pending)]);
        // Close denies it, and the helper is to exit.
        assert_eq!(rig.call(Call::Close), Ok(Reply::DoneThenExit));
        until("the denial", || rig.fake.seen.answer(100).is_some()).await;
        // The agent's end of the pipe is closed too: the session is really over.
        until("the agent to be cut off", || rig.fake.stopped()).await;
        assert!(
            denied(&rig.fake.seen.answer(100)),
            "{:?}",
            rig.fake.seen.answer(100)
        );
        assert_eq!(rig.cards(), [(1, Answer::Closed)]);
    });
}

fn a_request_too_large_to_review_is_denied_at_once() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        rig.fake.tell(Cmd::Ask {
            rpc_id: 101,
            command: "x".repeat(20_000),
        });
        until("the denial", || rig.fake.seen.answer(101).is_some()).await;
        assert!(
            denied(&rig.fake.seen.answer(101)),
            "{:?}",
            rig.fake.seen.answer(101)
        );
        assert_eq!(rig.cards(), [(1, Answer::TooLarge)]);
    });
}

fn the_shell_going_away_denies_what_is_pending_and_ends_the_session() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        rig.fake.tell(Cmd::Ask {
            rpc_id: 102,
            command: "touch y".into(),
        });
        until("the card", || rig.cards() == [(1, Answer::Pending)]).await;
        assert!(rig.service.shell_gone());
        until("the denial", || rig.fake.seen.answer(102).is_some()).await;
        assert!(denied(&rig.fake.seen.answer(102)));
        until("the agent to be cut off", || rig.fake.stopped()).await;
        // Nothing can be sent into it afterwards.
        assert_eq!(rig.send("more"), Err(HandlerError::NotReady));
        settle().await;
        assert!(rig.fake.seen.prompts().is_empty());
    });
}

fn the_shell_going_away_with_no_session_changes_nothing() {
    let rig = Rig::new(false);
    assert!(!rig.service.shell_gone());
    assert!(rig.service.conversation().is_none());
}

// Closing.

fn after_close_every_call_but_close_is_answered_not_ready_and_does_nothing() {
    let rig = Rig::new(false);
    run(async {
        rig.opened().await;
        assert_eq!(rig.call(Call::Close), Ok(Reply::DoneThenExit));
        let calls = [
            Call::Open,
            Call::Edited("x".into()),
            Call::EditLost,
            Call::Send {
                text: "x".into(),
                via: Via::Button,
            },
            Call::Cancel,
            Call::CheckUsage,
            Call::DraftChoice(Placement::Keep),
            Call::DraftDiscard,
        ];
        for call in calls {
            let name = call.name();
            assert_eq!(rig.call(call), Err(HandlerError::NotReady), "{name}");
        }
        assert_eq!(rig.call(Call::Close), Ok(Reply::DoneThenExit));
        settle().await;
        assert_eq!(rig.fake.starts(), 1);
        assert!(rig.fake.seen.prompts().is_empty());
        assert_eq!(rig.service.composer().text(), "");
    });
}

fn close_with_no_session_still_means_the_helper_exits() {
    let rig = Rig::new(false);
    assert_eq!(rig.call(Call::Close), Ok(Reply::DoneThenExit));
}

fn a_session_that_arrives_after_close_is_dropped_unused() {
    let rig = Rig::with(false, |fake| fake.delay = Duration::from_millis(300));
    run(async {
        assert_eq!(rig.call(Call::Open), Ok(Reply::Done));
        assert_eq!(rig.call(Call::Close), Ok(Reply::DoneThenExit));
        glib::timeout_future(Duration::from_millis(900)).await;
        assert!(!rig.ready());
        // The session that arrived late was dropped, not kept: the agent's end
        // of the pipe is closed.
        until("the late session to be dropped", || rig.fake.stopped()).await;
        assert_eq!(rig.send("late"), Err(HandlerError::NotReady));
        assert!(rig.fake.seen.prompts().is_empty());
    });
}

// The source.

#[test]
fn the_service_sends_a_prompt_from_one_place_only() {
    let source = include_str!("../src/service.rs");
    let code = source.split("#[cfg(test)]").next().unwrap();
    assert_eq!(code.matches(".prompt(").count(), 1, "one call to prompt");
}

macro_rules! scenarios {
    ($($name:ident),* $(,)?) => {
        #[test]
        fn the_service_against_a_fake_agent() {
            let mut failed = Vec::new();
            $(
                eprintln!("scenario: {}", stringify!($name));
                if std::panic::catch_unwind($name).is_err() {
                    failed.push(stringify!($name));
                }
            )*
            assert!(failed.is_empty(), "failed scenarios: {failed:?}");
        }
    };
}

scenarios!(
    open_starts_one_session_and_gets_it_ready_and_a_second_open_starts_none,
    the_launch_usage_goes_out_once_when_the_setting_is_on,
    no_launch_usage_when_the_setting_is_off,
    a_start_that_fails_leaves_a_conversation_that_cannot_send,
    send_puts_exactly_the_typed_text_in_one_prompt_and_empties_the_input,
    send_sends_its_own_text_not_the_last_edited_one,
    enter_sends_like_the_button_when_there_is_no_draft,
    nothing_is_sent_when_it_may_not_be_and_the_text_stays_in_the_input,
    a_message_the_core_refuses_goes_back_in_the_input,
    check_usage_sends_the_fixed_text_only_when_it_may,
    cancel_reaches_the_agent,
    the_draft_calls_change_nothing_while_there_is_no_draft,
    a_lost_edit_is_remembered_until_the_next_edit_and_a_send_makes_the_input_known,
    a_request_waits_unanswered_and_decide_and_shown_do_not_reach_it_until_close_denies_it,
    a_request_too_large_to_review_is_denied_at_once,
    the_shell_going_away_denies_what_is_pending_and_ends_the_session,
    the_shell_going_away_with_no_session_changes_nothing,
    close_with_no_session_still_means_the_helper_exits,
    after_close_every_call_but_close_is_answered_not_ready_and_does_nothing,
    a_session_that_arrives_after_close_is_dropped_unused,
);
