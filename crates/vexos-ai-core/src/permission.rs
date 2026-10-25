//! The approval options filter: of everything an agent offers, only "approve
//! once" and "deny once" survive. "Always" options (and anything unknown, such
//! as a plan-exit mode switch) are dropped here and never reach the UI.

use serde_json::{Value, json};

/// The only two answers the UI can give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    ApproveOnce,
    DenyOnce,
}

/// A permission request as the UI sees it.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    /// Pass back to [`crate::Session::decide`].
    pub id: u64,
    pub tool_call_id: Option<String>,
    pub title: String,
    pub kind: Option<String>,
    pub raw_input: Value,
    /// A subset of `[ApproveOnce, DenyOnce]`, in that order.
    pub choices: Vec<Decision>,
}

/// The agent's option ids for the two answers we allow, when it offered them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Options {
    approve_once: Option<String>,
    deny_once: Option<String>,
}

impl Options {
    pub(crate) fn from_acp(options: &Value) -> Self {
        let first = |kind: &str| {
            options.as_array()?.iter().find_map(|o| {
                (o["kind"].as_str() == Some(kind))
                    .then(|| o["optionId"].as_str().map(str::to_owned))?
            })
        };
        Options {
            approve_once: first("allow_once"),
            deny_once: first("reject_once"),
        }
    }

    pub(crate) fn choices(&self) -> Vec<Decision> {
        let mut choices = Vec::new();
        if self.approve_once.is_some() {
            choices.push(Decision::ApproveOnce);
        }
        if self.deny_once.is_some() {
            choices.push(Decision::DenyOnce);
        }
        choices
    }

    /// The response for `decision`, or `None` when it was not offered.
    pub(crate) fn outcome(&self, decision: Decision) -> Option<Value> {
        match decision {
            Decision::ApproveOnce => self.approve_once.as_deref().map(selected),
            Decision::DenyOnce => self.deny_once.as_deref().map(selected),
        }
    }

    /// Deny once, or cancel the request when the agent offered no way to deny.
    pub(crate) fn deny(&self) -> Value {
        self.outcome(Decision::DenyOnce).unwrap_or_else(cancelled)
    }
}

fn selected(option_id: &str) -> Value {
    json!({ "outcome": { "outcome": "selected", "optionId": option_id } })
}

pub(crate) fn cancelled() -> Value {
    json!({ "outcome": { "outcome": "cancelled" } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(kinds: &[(&str, &str)]) -> Value {
        kinds
            .iter()
            .map(|(id, kind)| json!({ "optionId": id, "name": id, "kind": kind }))
            .collect()
    }

    #[test]
    fn always_options_are_dropped() {
        let o = Options::from_acp(&opts(&[
            ("always", "allow_always"),
            ("yes", "allow_once"),
            ("never", "reject_always"),
            ("no", "reject_once"),
            ("switch", "switch_mode"),
        ]));
        assert_eq!(o.choices(), [Decision::ApproveOnce, Decision::DenyOnce]);
        assert_eq!(
            o.outcome(Decision::ApproveOnce).unwrap()["outcome"]["optionId"],
            "yes"
        );
        assert_eq!(o.deny()["outcome"]["optionId"], "no");
    }

    #[test]
    fn only_always_options_leave_nothing_to_approve() {
        let o = Options::from_acp(&opts(&[
            ("always", "allow_always"),
            ("never", "reject_always"),
        ]));
        assert!(o.choices().is_empty());
        assert_eq!(o.outcome(Decision::ApproveOnce), None);
        assert_eq!(o.deny(), cancelled());
    }

    #[test]
    fn malformed_options_offer_nothing() {
        for bad in [json!(null), json!("x"), json!([{ "kind": "allow_once" }])] {
            assert!(Options::from_acp(&bad).choices().is_empty(), "{bad}");
        }
    }
}
