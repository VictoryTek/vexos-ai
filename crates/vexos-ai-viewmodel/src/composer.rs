//! The input box. It holds what the user typed and any draft a notification
//! put there, and decides what may be sent.
//!
//! - [`Composer::submit`] is the only way to get an [`Outgoing`] message
//!   with typed text, and [`crate::Conversation::send`] takes nothing else.
//!   Holding, placing, editing and discarding a draft never produce one. The
//!   only other maker is [`crate::Conversation::check_usage`], whose text is
//!   the constant [`crate::USAGE_COMMAND`].
//! - A draft is placed only once the conversation can send. While it waits
//!   for review, Enter does nothing; only the Send button sends it. Once the
//!   user edits it, Enter works again, but it is still a draft (the banner
//!   stays) until it is sent or discarded.
//! - A draft never replaces text silently: if the input has text, the user
//!   chooses to replace it, add the draft below it, or keep their text.

use std::mem;

/// A message the user chose to send. Only [`Composer::submit`] (typed text)
/// and [`crate::Conversation::check_usage`] (the constant `/usage`) make one.
#[derive(Debug, PartialEq, Eq)]
pub struct Outgoing(pub(crate) String);

impl Outgoing {
    pub fn text(&self) -> &str {
        &self.0
    }

    pub fn into_text(self) -> String {
        self.0
    }
}

/// How the user asked to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    SendButton,
    EnterKey,
}

/// A draft in the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draft {
    /// As placed: only the Send button sends it.
    Pending,
    /// Changed by the user: Enter sends it too.
    Edited,
}

/// The user's answer when a draft arrives and the input has text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Replace,
    Append,
    Cancel,
}

/// What [`Composer::offer`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    Nothing,
    /// The draft is in the input now.
    Placed,
    /// The input has text: ask the user, then call [`Composer::resolve`].
    Ask,
}

#[derive(Debug, Default)]
pub struct Composer {
    text: String,
    draft: Option<Draft>,
    /// Waiting for the conversation to be able to send.
    held: Option<String>,
    /// Waiting for the user's [`Placement`].
    asking: Option<String>,
    /// The input from before a draft was added below it (its text, and
    /// whether it was itself a draft), for Discard.
    before: Option<(String, Option<Draft>)>,
}

impl Composer {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn draft(&self) -> Option<Draft> {
        self.draft
    }

    /// The "review, then click Send" banner (with Discard) is shown.
    pub fn banner(&self) -> bool {
        self.draft.is_some()
    }

    /// A draft has not reached the input yet.
    pub fn waiting(&self) -> bool {
        self.held.is_some() || self.asking.is_some()
    }

    /// A notification's draft. A newer one replaces one still waiting.
    pub fn hold(&mut self, draft: String) {
        self.held = Some(draft);
    }

    /// Places a waiting draft once the conversation can send. While the user
    /// is being asked, a newer draft replaces the one asked about.
    pub fn offer(&mut self, can_send: bool) -> Offer {
        if !can_send {
            return Offer::Nothing;
        }
        let Some(draft) = self.held.take() else {
            return Offer::Nothing;
        };
        if self.asking.is_some() {
            self.asking = Some(draft);
            return Offer::Nothing;
        }
        if self.text.trim().is_empty() {
            self.text = draft;
            self.draft = Some(Draft::Pending);
            Offer::Placed
        } else {
            self.asking = Some(draft);
            Offer::Ask
        }
    }

    /// The user's answer to [`Offer::Ask`]. Returns whether the text changed.
    pub fn resolve(&mut self, placement: Placement) -> bool {
        let Some(draft) = self.asking.take() else {
            return false;
        };
        match placement {
            Placement::Replace => self.text = draft,
            Placement::Append => {
                let typed = mem::take(&mut self.text);
                self.text = format!("{}\n\n{draft}", typed.trim_end());
                self.before = Some((typed, self.draft));
            }
            Placement::Cancel => return false,
        }
        self.draft = Some(Draft::Pending);
        true
    }

    /// The input's whole text after the user changed it. Clearing it all
    /// leaves nothing to review, so the draft is gone too.
    pub fn edited(&mut self, text: &str) {
        if text == self.text {
            return;
        }
        text.clone_into(&mut self.text);
        if self.text.trim().is_empty() {
            self.draft = None;
            self.before = None;
        } else if self.draft.is_some() {
            self.draft = Some(Draft::Edited);
        }
    }

    /// The Discard button: the draft goes. The input is cleared, or gets
    /// back the user's own text if the draft was added below it.
    pub fn discard(&mut self) -> bool {
        if self.draft.take().is_none() {
            return false;
        }
        (self.text, self.draft) = self.before.take().unwrap_or_default();
        true
    }

    /// The Send button or Enter. The message, and an empty input, when it may
    /// be sent; `None` (and nothing changes) when not.
    pub fn submit(&mut self, via: Via, can_send: bool) -> Option<Outgoing> {
        let blocked = via == Via::EnterKey && self.draft == Some(Draft::Pending);
        if !can_send || blocked || self.asking.is_some() || self.text.trim().is_empty() {
            return None;
        }
        self.draft = None;
        self.before = None;
        Some(Outgoing(mem::take(&mut self.text)))
    }

    /// A message that was not sent after all, back in the input as the
    /// user's own text. Text already there is kept, below it.
    pub fn restore(&mut self, text: String) {
        if self.text.trim().is_empty() {
            self.text = text;
            self.draft = None;
        } else {
            self.text = format!("{text}\n\n{}", self.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAFT: &str = "The systemd unit x.service failed.\n<<<BEGIN UNTRUSTED LOG OUTPUT>>>";

    fn placed() -> Composer {
        let mut c = Composer::default();
        c.hold(DRAFT.into());
        assert_eq!(c.offer(true), Offer::Placed);
        c
    }

    #[test]
    fn a_draft_waits_until_the_conversation_can_send() {
        let mut c = Composer::default();
        c.hold("first".into());
        c.hold("second".into());
        assert!(c.waiting());
        assert_eq!(c.offer(false), Offer::Nothing);
        assert_eq!(c.text(), "");
        assert_eq!(c.offer(true), Offer::Placed);
        assert_eq!(
            (c.text(), c.draft(), c.banner()),
            ("second", Some(Draft::Pending), true)
        );
        assert!(!c.waiting());
        assert_eq!(c.offer(true), Offer::Nothing);
    }

    #[test]
    fn enter_on_a_pending_draft_does_nothing() {
        let mut c = placed();
        assert_eq!(c.submit(Via::EnterKey, true), None);
        assert_eq!((c.text(), c.draft()), (DRAFT, Some(Draft::Pending)));
    }

    #[test]
    fn the_send_button_sends_a_pending_draft() {
        let mut c = placed();
        assert_eq!(c.submit(Via::SendButton, false), None);
        assert_eq!(c.draft(), Some(Draft::Pending));
        let out = c.submit(Via::SendButton, true).expect("sent");
        assert_eq!(out.text(), DRAFT);
        assert_eq!((c.text(), c.draft(), c.banner()), ("", None, false));
    }

    #[test]
    fn discard_clears_the_draft_and_the_banner() {
        let mut c = placed();
        assert!(c.discard());
        assert_eq!((c.text(), c.draft(), c.banner()), ("", None, false));
        assert!(!c.discard());
        // Typed text is not a draft: Discard leaves it alone.
        c.edited("mine");
        assert!(!c.discard());
        assert_eq!(c.text(), "mine");
    }

    #[test]
    fn an_edited_draft_keeps_its_banner_until_sent() {
        let mut c = placed();
        c.edited(&format!("{DRAFT}\nAlso: it started after the update."));
        assert_eq!((c.draft(), c.banner()), (Some(Draft::Edited), true));
        c.edited(&format!("{DRAFT} more"));
        assert_eq!((c.draft(), c.banner()), (Some(Draft::Edited), true));
        // Enter works again once edited, and sending clears the banner.
        let out = c.submit(Via::EnterKey, true).expect("sent");
        assert_eq!(out.text(), format!("{DRAFT} more"));
        assert!(!c.banner());
    }

    #[test]
    fn echoes_of_the_placed_text_are_not_edits() {
        let mut c = placed();
        c.edited(DRAFT);
        assert_eq!(c.draft(), Some(Draft::Pending));
        assert_eq!(c.submit(Via::EnterKey, true), None);
    }

    #[test]
    fn clearing_the_input_ends_the_draft() {
        let mut c = placed();
        c.edited("  ");
        assert_eq!((c.draft(), c.banner()), (None, false));
    }

    #[test]
    fn typed_text_sends_with_enter_as_before() {
        let mut c = Composer::default();
        c.edited("how big is swap?");
        assert_eq!(c.draft(), None);
        assert_eq!(c.submit(Via::EnterKey, false), None);
        assert_eq!(
            c.submit(Via::EnterKey, true).unwrap().text(),
            "how big is swap?"
        );
        c.edited("   ");
        assert_eq!(c.submit(Via::SendButton, true), None);
    }

    #[test]
    fn a_draft_never_replaces_typed_text_without_asking() {
        let mut c = Composer::default();
        c.edited("my question");
        c.hold(DRAFT.into());
        assert_eq!(c.offer(true), Offer::Ask);
        assert_eq!((c.text(), c.draft()), ("my question", None));
        // Nothing can be sent while the question is open.
        assert_eq!(c.submit(Via::SendButton, true), None);
        assert!(c.waiting());
    }

    #[test]
    fn replace_append_or_cancel() {
        let ask = || {
            let mut c = Composer::default();
            c.edited("my question\n");
            c.hold(DRAFT.into());
            assert_eq!(c.offer(true), Offer::Ask);
            c
        };
        let mut c = ask();
        assert!(c.resolve(Placement::Replace));
        assert_eq!((c.text(), c.draft()), (DRAFT, Some(Draft::Pending)));

        let mut c = ask();
        assert!(c.resolve(Placement::Append));
        assert_eq!(c.text(), format!("my question\n\n{DRAFT}"));
        assert_eq!(c.draft(), Some(Draft::Pending));
        assert_eq!(c.submit(Via::EnterKey, true), None);

        // Discarding an added draft gives the user's own text back.
        assert!(c.discard());
        assert_eq!((c.text(), c.draft()), ("my question\n", None));

        let mut c = ask();
        assert!(c.resolve(Placement::Replace));
        assert!(c.discard());
        assert_eq!(c.text(), "");

        let mut c = ask();
        assert!(!c.resolve(Placement::Cancel));
        assert_eq!(
            (c.text(), c.draft(), c.waiting()),
            ("my question\n", None, false)
        );
        assert!(!c.resolve(Placement::Replace));
        assert_eq!(
            c.submit(Via::EnterKey, true).unwrap().text(),
            "my question\n"
        );
    }

    #[test]
    fn a_draft_in_the_input_counts_as_text() {
        let mut c = placed();
        c.hold("newer".into());
        assert_eq!(c.offer(true), Offer::Ask);
        assert_eq!(c.text(), DRAFT);
    }

    #[test]
    fn a_newer_draft_while_asking_replaces_the_one_asked_about() {
        let mut c = Composer::default();
        c.edited("mine");
        c.hold("old".into());
        assert_eq!(c.offer(true), Offer::Ask);
        c.hold("new".into());
        assert_eq!(c.offer(true), Offer::Nothing);
        assert!(c.resolve(Placement::Replace));
        assert_eq!(c.text(), "new");
    }

    #[test]
    fn nothing_but_submit_makes_a_message() {
        // Every other operation leaves no message behind: the only `Outgoing`
        // values in this test come from `submit`.
        let mut c = Composer::default();
        c.hold(DRAFT.into());
        c.offer(true);
        c.edited(&format!("{DRAFT}!"));
        c.hold("again".into());
        c.offer(true);
        c.resolve(Placement::Append);
        // The first draft comes back as a draft, then goes.
        c.discard();
        assert_eq!(
            (c.text(), c.draft()),
            (format!("{DRAFT}!").as_str(), Some(Draft::Edited))
        );
        c.discard();
        assert_eq!((c.text(), c.draft()), ("", None));
        assert_eq!(c.submit(Via::SendButton, true), None);
    }

    #[test]
    fn a_refused_message_comes_back_as_typed_text() {
        let mut c = placed();
        let out = c.submit(Via::SendButton, true).unwrap();
        c.restore(out.into_text());
        assert_eq!((c.text(), c.draft()), (DRAFT, None));
        // Something already in the input is kept, below.
        let mut c = Composer::default();
        c.edited("typed meanwhile");
        c.restore("refused".into());
        assert_eq!(c.text(), "refused\n\ntyped meanwhile");
    }
}
