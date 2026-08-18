//! The transformations Bartizan offers.
//!
//! Each module exposes `fix(file, offset) -> Option<Fix>` and is responsible for
//! deciding it does not apply. Declining is the substance of this work: these
//! exist precisely because clippy judged the rewrites unsafe to apply blindly.

use ra_ap_syntax::{SourceFile, TextRange, TextSize};

pub mod collapsible_match;
pub mod unnecessary_cast;

/// One offered transformation.
pub struct Fix {
    pub title: String,
    /// Non-overlapping replacements, in no particular order.
    pub edits: Vec<(TextRange, String)>,
}

/// The transformation answering `code`, a lint name the editor reported over
/// `range`.
///
/// This is the other half of the server. [`all`] offers what the syntax tree can
/// prove on its own at a cursor; this offers what a type-aware checker has
/// already concluded, so between them a lint's whole surface is covered rather
/// than only the shapes provable without types.
pub fn for_diagnostic(file: &SourceFile, code: &str, range: TextRange) -> Option<Fix> {
    // Matched on the last path segment: rust-analyzer reports clippy lints as
    // `clippy::unnecessary_cast`, while other clients spell the same lint
    // without the tool prefix.
    match code.rsplit("::").next()? {
        "unnecessary_cast" => unnecessary_cast::from_clippy(file, range),
        _ => None,
    }
}

/// Every transformation applicable at `offset`.
pub fn all(file: &SourceFile, offset: TextSize) -> Vec<Fix> {
    [
        collapsible_match::fix(file, offset),
        unnecessary_cast::fix(file, offset),
    ]
    .into_iter()
    .flatten()
    .collect()
}
