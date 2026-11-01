//! Test-only: one valid call to each of the helper's methods.

use glib::prelude::ToVariant;

pub fn calls() -> Vec<(&'static str, Option<glib::Variant>)> {
    vec![
        ("Open", None),
        ("Close", None),
        ("Edited", Some(("hi",).to_variant())),
        ("Send", Some(("hi", "button").to_variant())),
        ("Cancel", None),
        ("CheckUsage", None),
        ("DraftChoice", Some(("keep",).to_variant())),
        ("DraftDiscard", None),
        ("Decide", Some((1u64, 2u64, "deny-once").to_variant())),
        ("Shown", Some((1u64, 2u64).to_variant())),
    ]
}
