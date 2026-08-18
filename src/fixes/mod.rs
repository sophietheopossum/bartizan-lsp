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
