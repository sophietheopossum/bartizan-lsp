//! Move a `#[cfg(test)]` module below the items that follow it — the
//! transformation `clippy::items_after_test_module` asks for.
//!
//! ```ignore
//! fn helper() {}                     fn helper() {}
//!
//! #[cfg(test)]                 =>    fn used_by_nobody_yet() {}
//! mod tests { .. }
//!                                    #[cfg(test)]
//! fn used_by_nobody_yet() {}         mod tests { .. }
//! ```
//!
//! Clippy has no suggestion here because a relocation is not a text edit it can
//! express — its machinery emits replacements at a span, and this needs a cut in
//! one place and a paste in another. That is the whole transformation: the item
//! text is moved verbatim, never reformatted. Being top-level, nothing needs
//! reindenting either, which is what makes it safe to do syntactically.
//!
//! The reordering cannot change what compiles. Items at module scope are not
//! order-dependent, and the one construct that is — `macro_rules!` — only moves
//! *earlier* relative to the tests, never later. A macro defined inside a
//! `#[cfg(test)]` module is not visible outside it, so items left above cannot
//! have been relying on one.

use ra_ap_syntax::ast::{self, AstNode, HasAttrs, HasModuleItem};
use ra_ap_syntax::{SourceFile, SyntaxKind, TextRange, TextSize};

use super::Fix;

pub fn fix(file: &SourceFile, offset: TextSize) -> Option<Fix> {
    let items: Vec<ast::Item> = file.items().collect();
    let index = items.iter().position(is_cfg_test_module)?;
    let test_module = items.get(index)?;
    let trailing = items.get(index + 1..)?;

    // Already last: nothing to answer.
    if trailing.is_empty() {
        return None;
    }

    // Offered only from the module itself or from one of the items stranded
    // after it — the two places clippy points at. Anywhere else in the file this
    // would be an unexplained entry in the menu.
    let module_range = test_module.syntax().text_range();
    let in_scope = module_range.contains_inclusive(offset)
        || trailing
            .iter()
            .any(|item| item.syntax().text_range().contains_inclusive(offset));
    if !in_scope {
        return None;
    }

    // A second `#[cfg(test)]` module below means there is no single answer to
    // "where does the test module go".
    if trailing.iter().any(is_cfg_test_module) {
        return None;
    }

    let next = trailing.first()?;
    let last = trailing.last()?;

    // The blank line after the module has to travel with it, or removing the
    // module leaves a widening gap behind. Only whitespace may be swallowed: a
    // comment sitting in that gap belongs to the item below and would be
    // destroyed, so decline rather than move someone's note.
    let gap = TextRange::new(module_range.end(), next.syntax().text_range().start());
    if contains_comment(file, gap) {
        return None;
    }

    let moved = test_module.syntax().text().to_string();

    Some(Fix {
        title: "Move the test module below the items that follow it".to_owned(),
        edits: vec![
            // Cut, including the whitespace that separated it from what follows.
            (
                TextRange::new(module_range.start(), gap.end()),
                String::new(),
            ),
            // Paste at the end of the last item — before its trailing newline, so
            // the file keeps exactly the ending it already had.
            (
                TextRange::empty(last.syntax().text_range().end()),
                format!("\n\n{moved}"),
            ),
        ],
    })
}

fn is_cfg_test_module(item: &ast::Item) -> bool {
    let ast::Item::Module(module) = item else {
        return false;
    };
    module.attrs().any(|attr| {
        attr.syntax()
            .text()
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            == "#[cfg(test)]"
    })
}

fn contains_comment(file: &SourceFile, range: TextRange) -> bool {
    if range.is_empty() {
        return false;
    }
    file.syntax().descendants_with_tokens().any(|element| {
        element.kind() == SyntaxKind::COMMENT && range.contains_range(element.text_range())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ra_ap_syntax::Edition;

    /// Applies the fix at the `$0` marker, or returns None if it declines.
    fn apply(marked: &str) -> Option<String> {
        let offset = TextSize::new(marked.find("$0").expect("no $0 marker") as u32);
        let text = marked.replace("$0", "");
        let parse = SourceFile::parse(&text, Edition::CURRENT);
        let fix = fix(&parse.tree(), offset)?;

        let mut edits = fix.edits;
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start()));
        let mut out = text;
        for (range, new) in edits {
            out.replace_range(usize::from(range.start())..usize::from(range.end()), &new);
        }
        Some(out)
    }

    fn declines(marked: &str) {
        assert_eq!(apply(marked), None, "expected the fix to decline");
    }

    #[test]
    fn moves_the_module_below_a_trailing_item() {
        let got = apply(
            "fn helper() {}\n\n#[cfg(test)]\nmod te$0sts {\n    #[test]\n    fn t() {}\n}\n\nfn later() {}\n",
        )
        .expect("should apply");
        assert_eq!(
            got,
            "fn helper() {}\n\nfn later() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n"
        );
    }

    #[test]
    fn offered_from_the_stranded_item_too() {
        // Clippy points at the item below as well as the module.
        let got = apply("#[cfg(test)]\nmod tests {}\n\nfn la$0ter() {}\n").expect("should apply");
        assert_eq!(got, "fn later() {}\n\n#[cfg(test)]\nmod tests {}\n");
    }

    #[test]
    fn moves_below_several_trailing_items() {
        let got = apply("#[cfg(test)]\nmod te$0sts {}\n\nfn a() {}\n\nfn b() {}\n")
            .expect("should apply");
        assert_eq!(
            got,
            "fn a() {}\n\nfn b() {}\n\n#[cfg(test)]\nmod tests {}\n"
        );
    }

    #[test]
    fn moves_the_module_verbatim() {
        // The point of the lint's fix: text is relocated, never rewritten.
        let body = "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn t() {\n        assert!(true);\n    }\n}";
        let got = apply(&format!(
            "fn a() {{}}\n\n{}\n\nfn b() {{}}\n",
            body.replace("mod tests", "mod te$0sts")
        ))
        .expect("applies");
        assert!(got.contains(body), "module text was altered:\n{got}");
    }

    #[test]
    fn declines_when_already_last() {
        declines("fn a() {}\n\n#[cfg(test)]\nmod te$0sts {}\n");
    }

    #[test]
    fn declines_from_an_item_above_the_module() {
        // Nothing is wrong at that cursor; the menu should stay quiet.
        declines("fn he$0lper() {}\n\n#[cfg(test)]\nmod tests {}\n\nfn later() {}\n");
    }

    #[test]
    fn declines_on_a_plain_module() {
        declines("mod hel$0pers {}\n\nfn later() {}\n");
    }

    #[test]
    fn declines_with_a_second_test_module_below() {
        declines("#[cfg(test)]\nmod te$0sts {}\n\nfn a() {}\n\n#[cfg(test)]\nmod more_tests {}\n");
    }

    #[test]
    fn a_comment_introducing_the_next_item_travels_with_it() {
        // Written directly above `later`, the comment is part of that item's
        // node, so it moves with it and the gap is pure whitespace.
        let got = apply("#[cfg(test)]\nmod te$0sts {}\n\n// explains later\nfn later() {}\n")
            .expect("should apply");
        assert_eq!(
            got,
            "// explains later\nfn later() {}\n\n#[cfg(test)]\nmod tests {}\n"
        );
    }

    #[test]
    fn declines_when_a_detached_comment_would_be_swallowed() {
        // A blank line below leaves the comment stranded in the gap rather than
        // attached to the item, so the cut would delete it.
        declines("#[cfg(test)]\nmod te$0sts {}\n\n// standalone\n\nfn later() {}\n");
    }

    #[test]
    fn declines_on_a_trailing_comment_after_the_module() {
        // ` // note` sits in the gap too, on the module's closing line.
        declines("#[cfg(test)]\nmod te$0sts {} // note\n\nfn later() {}\n");
    }
}
