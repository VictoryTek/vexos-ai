# Reading guide: `composer.rs`

Written for: the repo owner, reading the file once (plan item E1).

File: `crates/vexos-ai-viewmodel/src/composer.rs` (about 190 lines of code, then tests).

**What it decides.** What text the panel may send to the agent. If the code here is right, nothing the agent, a notification or a bug can type for you ever goes out without your click.

## The rule to check

Text typed by a person leaves the panel in exactly one place: `Composer::submit`. Everything else only moves text around the input box.

Two senders exist in total:

1. `Composer::submit` sends what you typed or reviewed.
2. `Conversation::check_usage` (in `conversation.rs`, near `USAGE_COMMAND`) sends only the constant `/usage`. It takes no argument.

## Read in this order

1. **Lines 1-14, the header comment.** The promise, in words. Everything below should keep it.
2. **`Outgoing` (line 21).** A message that may be sent. Its field is `pub(crate)`, so code outside the view-model crate cannot build one. Look for: any other place that writes `Outgoing(`. The panel's source scans enforce this (`scan_tests.rs`, around lines 111-135).
3. **`submit` (around line 172).** The one maker of an `Outgoing` from typed text. Check each condition that makes it return `None`:
   - `!can_send`: the conversation is not ready.
   - `blocked`: Enter on a draft you have not edited.
   - `self.asking.is_some()`: a "replace, add or keep?" question is open.
   - empty text.

   Only after all of those pass does it clear the input and return the text. Look for: a path where a draft could pass without a click.
4. **`hold` and `offer` (around lines 100-125).** How a notification's draft arrives. They only fill the input box. Look for: anything that returns an `Outgoing` here. Nothing should.
5. **`resolve` (Replace, Append, Cancel) and `discard`.** These change the box, never send. Check that Cancel and Discard leave your own text intact.
6. **`edited`.** Marks a draft as `Edited` once you change it. Enter then works, but the draft banner stays until you send or discard. Check that an echo of the same text (the toolkit repeating what was placed) is not counted as an edit; `echoes_of_the_placed_text_are_not_edits` tests this.
7. **`restore`.** Puts a refused message back as your own text. It must not send.

## Questions to ask while reading

- Can a draft ever be sent by Enter before you edit it? (`blocked` says no.)
- Can a newer draft overwrite text you typed without asking? (`offer` and `resolve` say no.)
- Is there any function that takes a `String` from outside and returns an `Outgoing`? (There should be none.)
- Does anything reach `submit` other than the Send button and the Enter key?

## What the tests cover

`nothing_but_submit_makes_a_message` and `enter_on_a_pending_draft_does_nothing` are the two that matter most. The rest cover draft placement and the replace, add or keep choice.

## Not covered by this file

- Whether Enter fires in the middle of an input-method composition (a Phase B item, B2, to test).
- The approval cards (`Decide`), which are in the core and `conversation.rs`.
- The source scans that guard this rule are in `crates/vexos-ai-panel/src/scan_tests.rs`. E4 sends a fresh reviewer to try to break the rule.
